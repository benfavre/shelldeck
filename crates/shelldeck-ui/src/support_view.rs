//! Support mode — a native two-pane helpdesk console over the token-gated
//! support API. Left: view filters + ticket list. Right: the conversation, a
//! reply/note composer, and an action bar (status / priority / assign / resolve).
//!
//! The view holds data and captures composer text; all network happens in the
//! `Workspace` (background executor) driven by [`SupportViewEvent`].

use crate::icons::lucide_icon;
use crate::scale::px;
use adabraka_ui::components::button::{Button, ButtonSize, ButtonVariant};
use adabraka_ui::components::checkbox::Checkbox;
use adabraka_ui::components::confirm_dialog::Dialog as UiDialog;
use adabraka_ui::components::icon_button::IconButton;
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::label::Label;
use adabraka_ui::components::select::{Select, SelectOption};
use adabraka_ui::overlays::popover_menu::{PopoverMenu, PopoverMenuItem};
use adabraka_ui::components::avatar::{Avatar, AvatarSize};
use adabraka_ui::components::input::{Input, InputSize, InputState};
use adabraka_ui::display::badge::{Badge, BadgeVariant};
use gpui::prelude::*;
use gpui::*;

use shelldeck_core::config::issues::{Issue, IssueInstance};
use shelldeck_core::config::manage_support::{
    SupportAgent, SupportCounts, SupportMe, SupportMessage, SupportTicket,
};

use crate::theme::ShellDeckColors;
use crate::t;

/// Which section of the support console is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportSection {
    Tickets,
    Requests,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportFilter {
    All,
    Unassigned,
    Mine,
    Open,
    Pending,
    Breaching,
    Closed,
}

impl SupportFilter {
    fn label(self) -> String {
        match self {
            SupportFilter::All => t!("support.filter.all"),
            SupportFilter::Unassigned => t!("support.filter.unassigned"),
            SupportFilter::Mine => t!("support.filter.mine"),
            SupportFilter::Open => t!("support.filter.open"),
            SupportFilter::Pending => t!("support.filter.pending"),
            SupportFilter::Breaching => t!("support.filter.breaching"),
            SupportFilter::Closed => t!("support.filter.closed"),
        }
        .to_string()
    }
    fn count(self, c: &SupportCounts) -> u32 {
        match self {
            SupportFilter::All => c.all,
            SupportFilter::Unassigned => c.unassigned,
            SupportFilter::Mine => c.mine,
            SupportFilter::Open => c.open,
            SupportFilter::Pending => c.pending,
            SupportFilter::Breaching => c.breaching,
            SupportFilter::Closed => c.closed,
        }
    }
    const ALL: [SupportFilter; 7] = [
        SupportFilter::All,
        SupportFilter::Unassigned,
        SupportFilter::Mine,
        SupportFilter::Open,
        SupportFilter::Pending,
        SupportFilter::Breaching,
        SupportFilter::Closed,
    ];
}

/// Advanced filter option — `value` is `None` for the "all" chip.
struct AdvChannelOpt {
    value: Option<&'static str>,
    icon: &'static str,
}

fn adv_channel_label(value: Option<&str>) -> String {
    match value {
        None => t!("support.channel.all"),
        Some("livechat") => t!("support.channel.chat"),
        Some("email") => t!("support.channel.email"),
        Some("sms") => t!("support.channel.sms"),
        Some("contact") => t!("support.channel.contact"),
        Some("manage") => t!("support.channel.manage"),
        _ => t!("support.channel.all"),
    }
    .to_string()
}

const ADV_CHANNELS: &[AdvChannelOpt] = &[
    AdvChannelOpt {
        value: None,
        icon: "inbox",
    },
    AdvChannelOpt {
        value: Some("livechat"),
        icon: "reply",
    },
    AdvChannelOpt {
        value: Some("email"),
        icon: "mail",
    },
    AdvChannelOpt {
        value: Some("sms"),
        icon: "send",
    },
    AdvChannelOpt {
        value: Some("contact"),
        icon: "user",
    },
    AdvChannelOpt {
        value: Some("manage"),
        icon: "server",
    },
];

struct AdvPriorityOpt {
    value: Option<&'static str>,
}

fn adv_priority_label(value: Option<&str>) -> String {
    match value {
        None => t!("support.priority.all"),
        Some("low") => t!("support.priority.low"),
        Some("normal") => t!("support.priority.normal"),
        Some("high") => t!("support.priority.high"),
        Some("urgent") => t!("support.priority.urgent"),
        _ => t!("support.priority.all"),
    }
    .to_string()
}

const ADV_PRIORITIES: &[AdvPriorityOpt] = &[
    AdvPriorityOpt { value: None },
    AdvPriorityOpt {
        value: Some("low"),
    },
    AdvPriorityOpt {
        value: Some("normal"),
    },
    AdvPriorityOpt {
        value: Some("high"),
    },
    AdvPriorityOpt {
        value: Some("urgent"),
    },
];

#[derive(Clone, Copy)]
enum AdvPickField {
    Channel,
    Priority,
}

/// Select sentinel values for the assignee draft picker (`Select<String>`).
const ASSIGNEE_SELECT_ALL: &str = "__all__";
const ASSIGNEE_SELECT_NONE: &str = "__none__";

/// Which surface opened the active ticket popover menu.
#[derive(Clone, Debug)]
enum SupportMenuKind {
    ConversationHeader,
    TicketList(String),
}

/// Requests the view raises for the workspace to service (all network).
#[derive(Debug, Clone)]
pub enum SupportViewEvent {
    Refresh,
    SelectTicket(String),
    /// Send the composer text as a reply (note=false) or internal note (note=true).
    Send {
        id: String,
        text: String,
        note: bool,
    },
    SetStatus {
        id: String,
        status: String,
    },
    SetPriority {
        id: String,
        priority: String,
    },
    Assign {
        id: String,
        assignee: String,
    },
    Resolve {
        id: String,
        resolution: String,
    },
    /// Confirm/reject a JeanClaude pending ticket from the Support strip.
    JeanConfirm(String),
    JeanReject(String),
    /// File the selected ticket to JeanClaude (the composed text via /api/say).
    SendToJean(String),
    /// Convert a support ticket into a tracked request (source="support").
    ConvertToIssue {
        title: String,
        body: String,
    },
    // ── Requests (issues) tab ──
    IssuesRefresh,
    SelectIssue(String),
    IssueComment {
        id: String,
        body: String,
    },
    IssueStatus {
        id: String,
        status: String,
    },
    IssueAssign {
        id: String,
        assignee: String,
    },
    IssuePriority {
        id: String,
        priority: String,
    },
    IssueDispatch {
        id: String,
        instance_id: String,
    },
    IssueGithubPush(String),
    IssueGithubRefresh(String),
}

impl EventEmitter<SupportViewEvent> for SupportView {}

pub struct SupportView {
    tickets: Vec<SupportTicket>,
    counts: SupportCounts,
    me: SupportMe,
    agents: Vec<SupportAgent>,
    selected_id: Option<String>,
    detail: Option<SupportTicket>,
    filter: SupportFilter,
    /// Text search over subject / contact / preview / id.
    search_state: Entity<InputState>,
    filter_modal_open: bool,
    adv_channel: Option<String>,
    adv_priority: Option<String>,
    adv_unread_only: bool,
    /// `None` = tous, `Some("")` = non assigné, `Some(email)` = agent.
    adv_assignee: Option<String>,
    adv_sla_only: bool,
    adv_draft_channel: Option<String>,
    adv_draft_priority: Option<String>,
    adv_draft_unread_only: bool,
    adv_draft_assignee: Option<String>,
    adv_draft_sla_only: bool,
    /// Assignee picker inside the filter dialog (adabraka-ui `Select`).
    assignee_draft_select: Entity<Select<String>>,
    /// Real `Input` state backing the reply / internal-note composer.
    composer_state: Entity<InputState>,
    compose_note: bool,
    loading: bool,
    error: Option<String>,
    assign_menu_open: bool,
    priority_menu_open: bool,
    /// Popover menu for ticket actions (header kebab or list row).
    popover_menu: Option<(SupportMenuKind, Point<Pixels>)>,
    // JeanClaude strip (fed by the workspace when Jean config is present).
    jean_available: bool,
    jean_pending: Vec<(String, String)>,
    jean_active: usize,
    // Requests (issues) tab, fed by the workspace.
    section: SupportSection,
    issues: Vec<Issue>,
    issues_staff: bool,
    issue_instances: Vec<IssueInstance>,
    issue_detail: Option<Issue>,
    issue_selected: Option<String>,
    issue_status_menu: bool,
    issue_assign_menu: bool,
    issue_dispatch_menu: bool,
    issue_priority_menu_open: bool,
    /// Header kebab menu for the open issue detail pane.
    issue_popover_menu: Option<Point<Pixels>>,
    issues_scroll: ScrollHandle,
    focus_handle: FocusHandle,
    /// Scroll handle for the messages pane. `set_detail` calls
    /// `scroll_to_bottom()` on it so opening a ticket lands the reader on
    /// the latest message (the classic chat/messaging behavior), not on
    /// the top of the history.
    messages_scroll: ScrollHandle,
}

impl SupportView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let parent = cx.entity();
        let assignee_draft_select =
            Self::build_assignee_draft_select(None, &[], parent, cx);
        Self {
            tickets: Vec::new(),
            counts: SupportCounts::default(),
            me: SupportMe::default(),
            agents: Vec::new(),
            selected_id: None,
            detail: None,
            filter: SupportFilter::All,
            search_state: cx.new(InputState::new),
            filter_modal_open: false,
            adv_channel: None,
            adv_priority: None,
            adv_unread_only: false,
            adv_assignee: None,
            adv_sla_only: false,
            adv_draft_channel: None,
            adv_draft_priority: None,
            adv_draft_unread_only: false,
            adv_draft_assignee: None,
            adv_draft_sla_only: false,
            assignee_draft_select,
            composer_state: cx.new(InputState::new),
            compose_note: false,
            loading: false,
            error: None,
            assign_menu_open: false,
            priority_menu_open: false,
            popover_menu: None,
            jean_available: false,
            jean_pending: Vec::new(),
            jean_active: 0,
            section: SupportSection::Tickets,
            issues: Vec::new(),
            issues_staff: false,
            issue_instances: Vec::new(),
            issue_detail: None,
            issue_selected: None,
            issue_status_menu: false,
            issue_assign_menu: false,
            issue_dispatch_menu: false,
            issue_priority_menu_open: false,
            issue_popover_menu: None,
            issues_scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            messages_scroll: ScrollHandle::new(),
        }
    }

    /// Switch the console section (palette / action shortcut to Demandes).
    pub fn set_section(&mut self, section: SupportSection) {
        self.section = section;
    }

    pub fn set_issues(&mut self, issues: Vec<Issue>, staff: bool, instances: Vec<IssueInstance>) {
        self.issues = issues;
        self.issues_staff = staff;
        self.issue_instances = instances;
    }

    pub fn set_issue_detail(&mut self, detail: Option<Issue>) {
        if let Some(d) = &detail {
            self.issue_selected = Some(d.id.clone());
        }
        self.issue_detail = detail;
        self.issue_popover_menu = None;
        self.issue_status_menu = false;
        self.issue_assign_menu = false;
        self.issue_dispatch_menu = false;
        self.issue_priority_menu_open = false;
        self.issues_scroll.scroll_to_bottom();
    }

    /// Feed the JeanClaude strip (workspace pushes this from the cached state).
    pub fn set_jean_brief(
        &mut self,
        available: bool,
        pending: Vec<(String, String)>,
        active: usize,
    ) {
        self.jean_available = available;
        self.jean_pending = pending;
        self.jean_active = active;
    }

    /// Compose the "Envoyer à Jean" text from the open ticket.
    fn jean_ticket_text(&self) -> Option<String> {
        let t = self.detail.as_ref()?;
        let last_customer = t
            .messages
            .iter()
            .rev()
            .find(|m| m.is_customer())
            .map(|m| m.text.clone())
            .unwrap_or_default();
        let truncated: String = last_customer.chars().take(500).collect();
        Some(format!(
            "[Ticket support {} — {}] {} — {}",
            t.id,
            t.contact.display(),
            if t.subject.trim().is_empty() {
                "(sans objet)"
            } else {
                t.subject.trim()
            },
            truncated
        ))
    }

    pub fn set_list(&mut self, tickets: Vec<SupportTicket>, counts: SupportCounts, me: SupportMe) {
        self.tickets = tickets;
        self.counts = counts;
        self.me = me;
        self.loading = false;
        self.error = None;
        // Keep the detail's slim fields in sync if the selected ticket moved.
        if let Some(id) = &self.selected_id {
            if let Some(updated) = self.tickets.iter().find(|t| &t.id == id).cloned() {
                if let Some(detail) = &mut self.detail {
                    let messages = std::mem::take(&mut detail.messages);
                    *detail = SupportTicket {
                        messages,
                        ..updated
                    };
                }
            }
        }
    }

    pub fn set_agents(&mut self, agents: Vec<SupportAgent>) {
        self.agents = agents;
    }

    pub fn has_agents(&self) -> bool {
        !self.agents.is_empty()
    }

    /// Install a freshly-fetched detail (with messages) for the selected ticket.
    ///
    /// Preserves the current thread when the incoming ticket has no messages:
    /// the Manage API's state-change endpoints (`support_assign`,
    /// `support_status`, `support_priority`, `support_resolve`) return only
    /// the meta ticket. Blindly replacing `self.detail` with that response
    /// wiped the conversation until the next full fetch. We keep the
    /// existing messages when the incoming payload is empty.
    pub fn set_detail(&mut self, ticket: SupportTicket, cx: &mut Context<Self>) {
        let preserved_msgs = if ticket.messages.is_empty() {
            self.detail
                .as_ref()
                .filter(|d| d.id == ticket.id)
                .map(|d| d.messages.clone())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let ticket = if !preserved_msgs.is_empty() {
            SupportTicket {
                messages: preserved_msgs,
                ..ticket
            }
        } else {
            ticket
        };
        // Merge the updated slim ticket into the list too (keeping any
        // messages we may have cached alongside).
        if let Some(existing) = self.tickets.iter_mut().find(|t| t.id == ticket.id) {
            let msgs = if !ticket.messages.is_empty() {
                ticket.messages.clone()
            } else {
                existing.messages.clone()
            };
            *existing = SupportTicket {
                messages: msgs,
                ..ticket.clone()
            };
        }
        self.selected_id = Some(ticket.id.clone());
        self.detail = Some(ticket);
        self.popover_menu = None;
        self.priority_menu_open = false;
        self.assign_menu_open = false;
        self.reset_composer(cx);
        self.loading = false;
        self.error = None;
        // Land on the latest message — every chat / messaging UX defaults
        // to bottom-of-thread on open, not top.
        self.messages_scroll.scroll_to_bottom();
    }

    fn reset_composer(&self, cx: &mut Context<Self>) {
        self.composer_state.update(cx, |s, cx| {
            s.content = "".into();
            cx.notify();
        });
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.error = Some(msg.into());
        self.loading = false;
    }

    pub fn selected_id(&self) -> Option<String> {
        self.selected_id.clone()
    }

    fn my_email(&self) -> &str {
        &self.me.email
    }

    fn passes_filter(&self, t: &SupportTicket) -> bool {
        match self.filter {
            SupportFilter::All => true,
            SupportFilter::Unassigned => t.is_unassigned(),
            SupportFilter::Mine => !self.my_email().is_empty() && t.assignee == self.me.email,
            SupportFilter::Open => t.status == "open",
            SupportFilter::Pending => t.status == "pending",
            SupportFilter::Breaching => t.sla.breaching,
            SupportFilter::Closed => t.status == "closed",
        }
    }

    fn search_query(&self, cx: &Context<Self>) -> String {
        self.search_state.read(cx).content().to_string()
    }

    fn has_advanced_filters(&self) -> bool {
        self.adv_unread_only
            || self.adv_sla_only
            || self.adv_channel.is_some()
            || self.adv_priority.is_some()
            || self.adv_assignee.is_some()
    }

    fn has_list_constraints(&self, cx: &Context<Self>) -> bool {
        !self.search_query(cx).trim().is_empty() || self.has_advanced_filters()
    }

    fn sync_filter_draft_from_applied(&mut self) {
        self.adv_draft_channel = self.adv_channel.clone();
        self.adv_draft_priority = self.adv_priority.clone();
        self.adv_draft_unread_only = self.adv_unread_only;
        self.adv_draft_assignee = self.adv_assignee.clone();
        self.adv_draft_sla_only = self.adv_sla_only;
    }

    fn open_filter_modal(&mut self, cx: &mut Context<Self>) {
        self.sync_filter_draft_from_applied();
        self.refresh_assignee_draft_select(cx);
        self.filter_modal_open = true;
        cx.notify();
    }

    fn assignee_select_value(draft: &Option<String>) -> String {
        match draft {
            None => ASSIGNEE_SELECT_ALL.to_string(),
            Some(email) if email.is_empty() => ASSIGNEE_SELECT_NONE.to_string(),
            Some(email) => email.clone(),
        }
    }

    fn assignee_from_select_value(value: &str) -> Option<String> {
        match value {
            ASSIGNEE_SELECT_ALL => None,
            ASSIGNEE_SELECT_NONE => Some(String::new()),
            other => Some(other.to_string()),
        }
    }

    fn build_assignee_draft_select(
        draft: Option<String>,
        agents: &[SupportAgent],
        parent: Entity<SupportView>,
        cx: &mut Context<SupportView>,
    ) -> Entity<Select<String>> {
        let mut options = vec![
            SelectOption::new(
                ASSIGNEE_SELECT_ALL.to_string(),
                t!("support.assignee.all").to_string(),
            )
            .with_icon("icons/lucide/users.svg"),
            SelectOption::new(
                ASSIGNEE_SELECT_NONE.to_string(),
                t!("support.assignee.unassigned").to_string(),
            )
            .with_icon("icons/lucide/user.svg"),
        ];
        for agent in agents {
            let label = if agent.name.trim().is_empty() {
                agent.email.clone()
            } else {
                agent.name.clone()
            };
            options.push(
                SelectOption::new(agent.email.clone(), label)
                    .with_icon("icons/lucide/user-check.svg"),
            );
        }
        let selected_value = Self::assignee_select_value(&draft);
        let selected_index = options
            .iter()
            .position(|o| o.value == selected_value);
        cx.new(|select_cx| {
            Select::new(select_cx)
                .options(options)
                .selected_index(selected_index)
                .placeholder(t!("support.assignee.placeholder").to_string())
                .on_change({
                    move |value, _window, cx| {
                        parent.update(cx, |this, cx| {
                            this.adv_draft_assignee = Self::assignee_from_select_value(value);
                            cx.notify();
                        });
                    }
                })
        })
    }

    fn refresh_assignee_draft_select(&mut self, cx: &mut Context<Self>) {
        let parent = cx.entity();
        self.assignee_draft_select = Self::build_assignee_draft_select(
            self.adv_draft_assignee.clone(),
            &self.agents,
            parent,
            cx,
        );
    }

    fn apply_filter_draft(&mut self, cx: &mut Context<Self>) {
        self.adv_channel = self.adv_draft_channel.clone();
        self.adv_priority = self.adv_draft_priority.clone();
        self.adv_unread_only = self.adv_draft_unread_only;
        self.adv_assignee = self.adv_draft_assignee.clone();
        self.adv_sla_only = self.adv_draft_sla_only;
        self.filter_modal_open = false;
        cx.notify();
    }

    fn close_filter_modal(&mut self, cx: &mut Context<Self>) {
        self.filter_modal_open = false;
        cx.notify();
    }

    fn reset_filter_draft(&mut self, cx: &mut Context<Self>) {
        self.adv_draft_channel = None;
        self.adv_draft_priority = None;
        self.adv_draft_unread_only = false;
        self.adv_draft_assignee = None;
        self.adv_draft_sla_only = false;
        self.refresh_assignee_draft_select(cx);
        cx.notify();
    }

    fn clear_advanced_filters(&mut self, cx: &mut Context<Self>) {
        self.adv_channel = None;
        self.adv_priority = None;
        self.adv_unread_only = false;
        self.adv_assignee = None;
        self.adv_sla_only = false;
        if self.filter_modal_open {
            self.reset_filter_draft(cx);
        }
        cx.notify();
    }

    fn adv_channel_icon(value: &str) -> &'static str {
        ADV_CHANNELS
            .iter()
            .find(|o| o.value == Some(value))
            .map(|o| o.icon)
            .unwrap_or("inbox")
    }

    fn adv_channel_label(value: &str) -> String {
        adv_channel_label(Some(value))
    }

    fn adv_priority_label(value: &str) -> String {
        adv_priority_label(Some(value))
    }

    fn assignee_filter_label(&self, email: &str) -> String {
        if email.is_empty() {
            return t!("support.assignee.unassigned").to_string();
        }
        self.agents
            .iter()
            .find(|a| a.email == email)
            .map(|a| {
                if a.name.trim().is_empty() {
                    a.email.clone()
                } else {
                    a.name.clone()
                }
            })
            .unwrap_or_else(|| email.to_string())
    }

    fn passes_advanced(&self, t: &SupportTicket, query: &str) -> bool {
        if self.adv_unread_only && !t.unread {
            return false;
        }
        if let Some(ref ch) = self.adv_channel {
            if t.channel != *ch {
                return false;
            }
        }
        if let Some(ref p) = self.adv_priority {
            if t.priority != *p {
                return false;
            }
        }
        if let Some(ref assignee) = self.adv_assignee {
            if assignee.is_empty() {
                if !t.is_unassigned() {
                    return false;
                }
            } else if t.assignee != *assignee {
                return false;
            }
        }
        if self.adv_sla_only && !t.sla.breaching && !t.sla.breached {
            return false;
        }
        let q = query.trim();
        if q.is_empty() {
            return true;
        }
        let q = q.to_lowercase();
        let hay = format!(
            "{} {} {} {} {}",
            t.subject,
            t.contact.display(),
            t.contact.email.as_deref().unwrap_or(""),
            t.last_preview,
            t.id,
        )
        .to_lowercase();
        hay.contains(&q)
    }

    /// Read the composer content once and, if non-empty, emit the right event
    /// (reply / note / issue comment). Called from `Input::on_enter` and the
    /// send button.
    pub fn send_composer(&mut self, cx: &mut Context<Self>) {
        let text = self.composer_state.read(cx).content().trim().to_string();
        if text.is_empty() {
            return;
        }
        match self.section {
            SupportSection::Tickets => {
                if let Some(id) = self.selected_id.clone() {
                    let note = self.compose_note;
                    self.loading = true;
                    cx.emit(SupportViewEvent::Send { id, text, note });
                    cx.notify();
                }
            }
            SupportSection::Requests => {
                if let Some(id) = self.issue_selected.clone() {
                    self.reset_composer(cx);
                    cx.emit(SupportViewEvent::IssueComment { id, body: text });
                    cx.notify();
                }
            }
        }
    }

    // ── render helpers ───────────────────────────────────────────────────

    /// Compact JeanClaude strip: pending confirmations (confirm/reject inline)
    /// + active-ticket count. Shown only when Jean config is present.
    fn render_jean_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut strip = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .px(px(10.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("support.jean.title").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("support.jean.active", count = self.jean_active).to_string()),
                    ),
            );

        if self.jean_pending.is_empty() {
            strip = strip.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.jean.no_pending").to_string()),
            );
        } else {
            for (thread, prompt) in self.jean_pending.iter().take(4) {
                let t_ok = thread.clone();
                let t_no = thread.clone();
                let preview: String = prompt.chars().take(40).collect();
                strip = strip.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(
                            div()
                                .flex_1()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_primary())
                                .child(preview),
                        )
                        .child(
                            div()
                                .id(ElementId::from(SharedString::from(format!(
                                    "sj-ok-{thread}"
                                ))))
                                .px(px(5.0))
                                .rounded(px(4.0))
                                .bg(ShellDeckColors::success())
                                .text_size(px(11.0))
                                .text_color(white())
                                .cursor_pointer()
                                .child("✓")
                                .on_click(cx.listener(move |_t, _: &ClickEvent, _, cx| {
                                    cx.emit(SupportViewEvent::JeanConfirm(t_ok.clone()))
                                })),
                        )
                        .child(
                            div()
                                .id(ElementId::from(SharedString::from(format!(
                                    "sj-no-{thread}"
                                ))))
                                .px(px(5.0))
                                .rounded(px(4.0))
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::error())
                                .cursor_pointer()
                                .child("✕")
                                .on_click(cx.listener(move |_t, _: &ClickEvent, _, cx| {
                                    cx.emit(SupportViewEvent::JeanReject(t_no.clone()))
                                })),
                        ),
                );
            }
        }
        strip
    }

    /// Compact adabraka `Button` for the support filter strip / modal (Sm is 36px tall by default).
    fn compact_filter_button(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
    ) -> Button {
        Button::new(id, label)
            .size(ButtonSize::Sm)
            .h(gpui::px(26.0))
            .px(gpui::px(8.0))
    }

    fn render_pick_button(
        &self,
        cx: &mut Context<Self>,
        id: String,
        label: String,
        icon: &str,
        active: bool,
        field: AdvPickField,
        pick: Option<String>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let btn_id = ElementId::from(SharedString::from(id));
        Self::compact_filter_button(btn_id, label)
            .variant(ButtonVariant::Outline)
            .selected(active)
            .icon(IconSource::from(icon))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    match field {
                        AdvPickField::Channel => this.adv_draft_channel = pick.clone(),
                        AdvPickField::Priority => this.adv_draft_priority = pick.clone(),
                    }
                    cx.notify();
                });
            })
    }

    fn render_modal_pick_row(
        &self,
        cx: &mut Context<Self>,
        title: impl Into<SharedString>,
        id_prefix: &str,
        options: &[(String, Option<&str>, &str)],
        active: Option<&str>,
        field: AdvPickField,
    ) -> impl IntoElement {
        if matches!(field, AdvPickField::Priority) {
            let entity = cx.entity();
            let mut chips = div().flex().flex_wrap().gap(px(6.0));
            for (label, value, _) in options {
                let is_active = match (*value, active) {
                    (None, None) => true,
                    (Some(v), Some(a)) => v == a,
                    _ => false,
                };
                let pick = value.map(|s| s.to_string());
                let chip_id = format!("{id_prefix}-{}", label.replace(' ', "-"));
                let entity = entity.clone();
                let mut chip = div()
                    .id(ElementId::from(SharedString::from(chip_id)))
                    .p(px(2.0))
                    .rounded_full()
                    .cursor_pointer()
                    .border_2()
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.adv_draft_priority = pick.clone();
                            cx.notify();
                        });
                    });
                if let Some(v) = value {
                    chip = chip.child(priority_badge(v));
                } else {
                    chip = chip.child(Badge::new(label.to_string()).variant(BadgeVariant::Outline));
                }
                if is_active {
                    chip = chip.border_color(ShellDeckColors::primary());
                } else {
                    chip = chip.border_color(gpui::transparent_black()).opacity(0.55);
                }
                chips = chips.child(chip);
            }
            return div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(Label::new(title.into()))
                .child(chips);
        }

        let mut chips = div().flex().flex_wrap().gap(px(6.0));
        for (label, value, icon) in options {
            let is_active = match (*value, active) {
                (None, None) => true,
                (Some(v), Some(a)) => v == a,
                _ => false,
            };
            let pick = value.map(|s| s.to_string());
            chips = chips.child(self.render_pick_button(
                cx,
                format!("{id_prefix}-{}", label.replace(' ', "-")),
                label.to_string(),
                icon,
                is_active,
                field,
                pick,
            ));
        }
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .child(Label::new(title.into()))
            .child(chips)
    }

    fn render_applied_filter_chip(
        &self,
        id: String,
        icon: &str,
        label: String,
        cx: &mut Context<Self>,
        on_clear: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let entity = cx.entity();
        div()
            .id(ElementId::from(SharedString::from(id.clone())))
            .flex()
            .items_center()
            .gap(px(2.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(lucide_icon(icon, 11.0, ShellDeckColors::primary()))
                    .child(Badge::new(label).variant(BadgeVariant::Outline)),
            )
            .child(
                IconButton::new("x")
                    .variant(ButtonVariant::Ghost)
                    .size(gpui::px(28.0))
                    .icon_size(gpui::px(12.0))
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            on_clear(this, cx);
                        });
                    }),
            )
    }

    fn render_applied_filter_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div()
            .flex()
            .flex_wrap()
            .gap(px(4.0))
            .px(px(10.0))
            .pb(px(6.0));

        if let Some(ref ch) = self.adv_channel {
            let label = Self::adv_channel_label(ch);
            let icon = Self::adv_channel_icon(ch);
            row = row.child(self.render_applied_filter_chip(
                "applied-ch".to_string(),
                icon,
                label,
                cx,
                |this, cx| {
                    this.adv_channel = None;
                    cx.notify();
                },
            ));
        }
        if let Some(ref pr) = self.adv_priority {
            let label = Self::adv_priority_label(pr);
            row = row.child(self.render_applied_filter_chip(
                "applied-pr".to_string(),
                "flag",
                label,
                cx,
                |this, cx| {
                    this.adv_priority = None;
                    cx.notify();
                },
            ));
        }
        if self.adv_unread_only {
            row = row.child(self.render_applied_filter_chip(
                "applied-unread".to_string(),
                "eye",
                t!("support.chip.unread").to_string(),
                cx,
                |this, cx| {
                    this.adv_unread_only = false;
                    cx.notify();
                },
            ));
        }
        if let Some(ref assignee) = self.adv_assignee {
            let label = self.assignee_filter_label(assignee);
            row = row.child(self.render_applied_filter_chip(
                "applied-assignee".to_string(),
                "user-check",
                label,
                cx,
                |this, cx| {
                    this.adv_assignee = None;
                    cx.notify();
                },
            ));
        }
        if self.adv_sla_only {
            row = row.child(self.render_applied_filter_chip(
                "applied-sla".to_string(),
                "triangle-alert",
                t!("support.chip.sla_breach").to_string(),
                cx,
                |this, cx| {
                    this.adv_sla_only = false;
                    cx.notify();
                },
            ));
        }

        row
    }

    /// Filter dialog — adabraka-ui `confirm_dialog::Dialog` + form controls.
    fn render_filter_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let draft_channel = self.adv_draft_channel.as_deref();
        let draft_priority = self.adv_draft_priority.as_deref();
        let draft_unread = self.adv_draft_unread_only;
        let draft_sla = self.adv_draft_sla_only;

        let channel_opts: Vec<(String, Option<&str>, &str)> = ADV_CHANNELS
            .iter()
            .map(|o| (adv_channel_label(o.value), o.value, o.icon))
            .collect();
        let priority_opts: Vec<(String, Option<&str>, &str)> = ADV_PRIORITIES
            .iter()
            .map(|o| (adv_priority_label(o.value), o.value, "flag"))
            .collect();

        UiDialog::new()
            .width(gpui::px(380.0))
            .on_backdrop_click({
                let entity = entity.clone();
                move |_, cx| {
                    entity.update(cx, |this, cx| this.close_filter_modal(cx));
                }
            })
            .header(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(8.0))
                    .px(px(14.0))
                    .py(px(12.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(lucide_icon(
                                "filter",
                                14.0,
                                ShellDeckColors::text_primary(),
                            ))
                            .child(
                                div()
                                    .text_size(px(15.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(t!("support.filters.title").to_string()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(
                                Self::compact_filter_button(
                                    "filter-modal-reset",
                                    t!("support.filters.reset").to_string(),
                                )
                                    .variant(ButtonVariant::Ghost)
                                    .on_click({
                                        let entity = entity.clone();
                                        move |_, _, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.reset_filter_draft(cx);
                                                this.refresh_assignee_draft_select(cx);
                                            });
                                        }
                                    }),
                            )
                            .child(
                                IconButton::new("x")
                                    .variant(ButtonVariant::Ghost)
                                    .size(gpui::px(28.0))
                                    .icon_size(gpui::px(12.0))
                                    .on_click({
                                        let entity = entity.clone();
                                        move |_, _, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.close_filter_modal(cx);
                                            });
                                        }
                                    }),
                            ),
                    ),
            )
            .content(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(16.0))
                    .px(px(14.0))
                    .py(px(14.0))
                    .child(self.render_modal_pick_row(
                        cx,
                        t!("support.filter.channel").to_string(),
                        "modal-ch",
                        &channel_opts,
                        draft_channel,
                        AdvPickField::Channel,
                    ))
                    .child(self.render_modal_pick_row(
                        cx,
                        t!("support.filter.priority").to_string(),
                        "modal-pr",
                        &priority_opts,
                        draft_priority,
                        AdvPickField::Priority,
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(8.0))
                            .child(Label::new(t!("support.filter.assignee").to_string()))
                            .child(self.assignee_draft_select.clone()),
                    )
                    .child(
                        Checkbox::new("adv-draft-unread")
                            .checked(draft_unread)
                            .label(t!("support.filter.unread_only").to_string())
                            .on_click({
                                let entity = entity.clone();
                                move |checked, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.adv_draft_unread_only = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    )
                    .child(
                        Checkbox::new("adv-draft-sla")
                            .checked(draft_sla)
                            .label(t!("support.filter.sla_only").to_string())
                            .on_click({
                                let entity = entity.clone();
                                move |checked, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.adv_draft_sla_only = *checked;
                                        cx.notify();
                                    });
                                }
                            }),
                    ),
            )
            .footer(
                div()
                    .px(px(14.0))
                    .py(px(12.0))
                    .border_t_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        Self::compact_filter_button(
                            "filter-modal-apply",
                            t!("support.filters.apply").to_string(),
                        )
                            .variant(ButtonVariant::Default)
                            .icon(IconSource::from("check"))
                            .w_full()
                            .on_click({
                                let entity = entity.clone();
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.apply_filter_draft(cx);
                                    });
                                }
                            }),
                    ),
            )
    }

    fn render_filters(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let active_adv_count = [
            self.adv_channel.is_some(),
            self.adv_priority.is_some(),
            self.adv_unread_only,
            self.adv_assignee.is_some(),
            self.adv_sla_only,
        ]
        .iter()
        .filter(|&&b| b)
        .count();

        let filter_btn = IconButton::new("filter")
            .variant(if active_adv_count > 0 {
                ButtonVariant::Default
            } else {
                ButtonVariant::Outline
            })
            .size(gpui::px(28.0))
            .icon_size(gpui::px(12.0))
            .on_click({
                let entity = entity.clone();
                move |_, _, cx| {
                    entity.update(cx, |this, cx| this.open_filter_modal(cx));
                }
            });

        let search_row = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .pt(px(8.0))
            .pb(px(6.0))
            .child(
                div()
                    .flex_1()
                    .child(
                        Input::new(&self.search_state)
                            .size(InputSize::Sm)
                            .placeholder(t!("support.search_placeholder").to_string())
                            .prefix(lucide_icon(
                                "search",
                                12.0,
                                ShellDeckColors::text_muted(),
                            ))
                            .on_change({
                                let entity = entity.clone();
                                move |_, cx| {
                                    entity.update(cx, |_, cx| cx.notify());
                                }
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(filter_btn)
                    .when(active_adv_count > 0, |el| {
                        el.child(
                            Badge::new(active_adv_count.to_string())
                                .variant(BadgeVariant::Default),
                        )
                    }),
            );

        let mut chips_row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(4.0))
            .px(px(10.0))
            .pb(px(6.0));
        for f in SupportFilter::ALL {
            let active = self.filter == f;
            let count = f.count(&self.counts);
            let filter = f;
            chips_row = chips_row.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        Self::compact_filter_button(
                            ElementId::from(SharedString::from(format!("sf-{}", f.label()))),
                            f.label(),
                        )
                        .variant(ButtonVariant::Outline)
                        .selected(active)
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                entity.update(cx, |this, cx| {
                                    this.filter = filter;
                                    cx.notify();
                                });
                            }
                        }),
                    )
                    .child(
                        Badge::new(count.to_string()).variant(BadgeVariant::Secondary),
                    ),
            );
        }

        let mut panel = div()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(search_row)
            .child(chips_row);

        if self.has_advanced_filters() {
            panel = panel.child(self.render_applied_filter_chips(cx));
        }

        panel
    }
    fn render_ticket_row(&self, t: &SupportTicket, cx: &mut Context<Self>) -> impl IntoElement {
        let id_click = t.id.clone();
        let id_rclick = t.id.clone();
        let id_kebab = t.id.clone();
        let selected = self.selected_id.as_deref() == Some(t.id.as_str());
        let subject = if t.subject.trim().is_empty() {
            "(sans objet)".to_string()
        } else {
            t.subject.clone()
        };
        let group_name = SharedString::from(format!("tk-row-{}", t.id));

        let mut row = div()
            .id(ElementId::from(SharedString::from(format!("tk-{}", t.id))))
            .group(group_name.clone())
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px(px(10.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .cursor_pointer()
            .hover(|s| s.bg(ShellDeckColors::hover_bg()))
            .on_click(cx.listener(move |_this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                cx.emit(SupportViewEvent::SelectTicket(id_click.clone()));
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                    this.popover_menu = Some((
                        SupportMenuKind::TicketList(id_rclick.clone()),
                        event.position,
                    ));
                    cx.notify();
                }),
            );
        if selected {
            row = row.bg(ShellDeckColors::selected_bg());
        }

        let kebab = div()
            .id(ElementId::from(SharedString::from(format!("tk-kebab-{}", t.id))))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .w(px(22.0))
            .h(px(22.0))
            .rounded(px(4.0))
            .text_color(ShellDeckColors::text_muted())
            .opacity(0.35)
            .group_hover(group_name, |el| el.opacity(1.0))
            .cursor_pointer()
            .hover(|el| {
                el.bg(ShellDeckColors::hover_bg())
                    .text_color(ShellDeckColors::text_primary())
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                this.popover_menu = Some((
                    SupportMenuKind::TicketList(id_kebab.clone()),
                    event.position(),
                ));
                cx.notify();
            }))
            .child(lucide_icon("ellipsis-vertical", 14.0, ShellDeckColors::text_muted()));

        // Line 1: channel glyph + subject + priority dot + time + kebab
        let subject_weight = if t.unread {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::NORMAL
        };
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(lucide_icon(
                    t.channel_lucide(),
                    12.0,
                    ShellDeckColors::text_muted(),
                ))
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(13.0))
                        .font_weight(subject_weight)
                        .text_color(ShellDeckColors::text_primary())
                        .child(subject),
                )
                .child(priority_badge(&t.priority))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(rel_time(t.last_at)),
                )
                .child(kebab),
        );
        // Line 2: contact + preview
        row = row.child(
            div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t.contact.display()),
                )
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t.last_preview.clone()),
                ),
        );
        row
    }

    fn render_message(msg: &SupportMessage, me: &SupportMe) -> impl IntoElement {
        let (bg, align_end, label) = if msg.is_note() {
            (
                ShellDeckColors::warning().opacity(0.12),
                false,
                t!("support.note_internal").to_string(),
            )
        } else if msg.is_customer() {
            (
                ShellDeckColors::bg_surface(),
                false,
                t!("support.bubble.client").to_string(),
            )
        } else {
            (
                ShellDeckColors::primary().opacity(0.12),
                true,
                t!("support.bubble.agent").to_string(),
            )
        };
        // Fallback for the sender label: `msg.name` first (Manage API sets
        // it for messages typed from the web dashboard), then — for
        // agent-side messages with no name — the currently signed-in
        // agent's own name/email (this console is mono-agent, so a
        // nameless agent-side message is always ours). Notes and customer
        // messages keep the generic label.
        let who = msg
            .name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| {
                if !msg.is_note() && !msg.is_customer() {
                    let name = me.name.trim();
                    if !name.is_empty() {
                        Some(name.to_string())
                    } else {
                        let email = me.email.trim();
                        if !email.is_empty() {
                            Some(email.to_string())
                        } else {
                            None
                        }
                    }
                } else {
                    None
                }
            })
            .unwrap_or(label);

        // Bubble: `max_w(560)` caps the pill width; leaving the width
        // otherwise unconstrained lets the flex parent (`justify_end` on
        // the wrap when this is an agent-side message) push the bubble to
        // the correct edge. `min_w_0` + `w_full` on the text child were
        // added earlier to force horizontal wrap, but they made the bubble
        // stretch past its cap and broke the right-alignment for agent
        // messages — reverted to the pre-SDPATCH-011-hotfix layout.
        let bubble = div()
            .max_w(px(560.0))
            .rounded(px(8.0))
            .bg(bg)
            .border_1()
            .border_color(ShellDeckColors::border())
            .px(px(10.0))
            .py(px(7.0))
            .flex()
            .flex_col()
            .gap(px(3.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(10.0))
                    .child(
                        div()
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(who),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(rel_time(msg.at)),
                    ),
            )
            .child({
                // Split by hard newlines and give each line its own div
                // with a `max_w`. gpui's text element uses
                // `available_space.width` as `wrap_width` when the parent
                // constrains it; `max_w` on a per-line wrapper feeds a
                // Definite width down to `shape_text` so long lines wrap
                // to the right height, while short lines' wrappers stay
                // as narrow as their content. Result: bubble auto-sizes
                // to the widest actual line, capped at max_w, with a
                // correct measured height (no more bleed past the border).
                let mut body = div()
                    .flex()
                    .flex_col()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary());
                for line in msg.text.split('\n') {
                    let display: SharedString = if line.is_empty() {
                        " ".into()
                    } else {
                        line.to_string().into()
                    };
                    body = body.child(div().max_w(px(540.0)).child(display));
                }
                body
            });

        let mut wrap = div().w_full().flex();
        if align_end {
            wrap = wrap.justify_end();
        }
        wrap.child(bubble)
    }

    fn action_button(
        &self,
        id: &'static str,
        label: String,
        icon: Option<&'static str>,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let mut btn = div()
            .id(ElementId::from(SharedString::from(id.to_string())))
            .px(px(9.0))
            .py(px(5.0))
            .rounded(px(6.0))
            .border_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_primary())
            .text_size(px(12.0))
            .text_color(ShellDeckColors::text_primary())
            .cursor_pointer()
            .hover(|s| s.bg(ShellDeckColors::hover_bg()));
        if let Some(icon_name) = icon {
            btn = btn
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(lucide_icon(icon_name, 12.0, ShellDeckColors::text_muted()))
                .child(label);
        } else {
            btn = btn.child(label);
        }
        btn.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| on_click(this, cx)))
    }

    fn close_popover_menu(&mut self, cx: &mut Context<Self>) {
        self.popover_menu = None;
        cx.notify();
    }

    fn ticket_for_menu<'a>(&'a self, kind: &SupportMenuKind) -> Option<&'a SupportTicket> {
        match kind {
            SupportMenuKind::ConversationHeader => self.detail.as_ref(),
            SupportMenuKind::TicketList(id) => self.tickets.iter().find(|t| &t.id == id),
        }
    }

    fn jean_text_for_ticket(t: &SupportTicket) -> String {
        let truncated: String = t.last_preview.chars().take(500).collect();
        format!(
            "[Ticket support {} — {}] {} — {}",
            t.id,
            t.contact.display(),
            if t.subject.trim().is_empty() {
                "(sans objet)"
            } else {
                t.subject.trim()
            },
            truncated
        )
    }

    fn build_ticket_menu_items(
        &self,
        kind: &SupportMenuKind,
        entity: Entity<SupportView>,
    ) -> Vec<PopoverMenuItem> {
        let Some(ticket) = self.ticket_for_menu(kind) else {
            return vec![];
        };
        let id = ticket.id.clone();
        let is_pending = ticket.status == "pending";
        let is_mine = !self.my_email().is_empty()
            && ticket.assignee.eq_ignore_ascii_case(self.my_email());
        let (status_next, menu_status_label) = if is_pending {
            (
                "open".to_string(),
                t!("support.menu.reopen").to_string(),
            )
        } else {
            (
                "pending".to_string(),
                t!("support.menu.pending").to_string(),
            )
        };

        let mut items = Vec::new();

        if matches!(kind, SupportMenuKind::TicketList(_)) {
            let tid = id.clone();
            items.push(
                PopoverMenuItem::new("menu-open", t!("support.menu.open").to_string())
                    .icon("external-link")
                    .on_click({
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_popover_menu(cx);
                                cx.emit(SupportViewEvent::SelectTicket(tid.clone()));
                            });
                        }
                    }),
            );
        }

        {
            let sid = id.clone();
            let snext = status_next.clone();
            items.push(
                PopoverMenuItem::new("menu-status", menu_status_label)
                    .icon(if is_pending {
                        "circle-check"
                    } else {
                        "clock"
                    })
                    .on_click({
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_popover_menu(cx);
                                cx.emit(SupportViewEvent::SetStatus {
                                    id: sid.clone(),
                                    status: snext.clone(),
                                });
                            });
                        }
                    }),
            );
        }

        if !is_mine {
            let aid = id.clone();
            items.push(
                PopoverMenuItem::new("menu-assign-me", t!("support.menu.assign_me").to_string())
                    .icon("user-check")
                    .on_click({
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_popover_menu(cx);
                                cx.emit(SupportViewEvent::Assign {
                                    id: aid.clone(),
                                    assignee: "me".to_string(),
                                });
                            });
                        }
                    }),
            );
        }

        if matches!(kind, SupportMenuKind::ConversationHeader) {
            items.push(
                PopoverMenuItem::new("menu-priority", t!("support.menu.priority").to_string())
                    .icon("flag")
                    .on_click({
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_popover_menu(cx);
                                this.priority_menu_open = true;
                                this.assign_menu_open = false;
                                cx.notify();
                            });
                        }
                    }),
            );
            items.push(
                PopoverMenuItem::new("menu-assign", t!("support.menu.assign").to_string())
                    .icon("users")
                    .on_click({
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_popover_menu(cx);
                                this.assign_menu_open = true;
                                this.priority_menu_open = false;
                                cx.notify();
                            });
                        }
                    }),
            );
        } else {
            for p in ["low", "normal", "high", "urgent"] {
                let pid = id.clone();
                let plabel =
                    t!("support.menu.priority_set", priority = priority_label(p)).to_string();
                items.push(
                    PopoverMenuItem::new(format!("menu-prio-{p}"), plabel)
                        .icon("flag")
                        .on_click({
                            let entity = entity.clone();
                            let p = p.to_string();
                            move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    this.close_popover_menu(cx);
                                    cx.emit(SupportViewEvent::SetPriority {
                                        id: pid.clone(),
                                        priority: p.clone(),
                                    });
                                });
                            }
                        }),
                );
            }
        }

        if self.jean_available {
            let jean_text = if matches!(kind, SupportMenuKind::ConversationHeader) {
                self.jean_ticket_text()
            } else {
                Some(Self::jean_text_for_ticket(ticket))
            };
            if let Some(text) = jean_text {
                items.push(
                    PopoverMenuItem::new("menu-jean", t!("support.menu.jean").to_string())
                        .icon("send")
                        .on_click({
                            let entity = entity.clone();
                            move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    this.close_popover_menu(cx);
                                    cx.emit(SupportViewEvent::SendToJean(text.clone()));
                                });
                            }
                        }),
                );
            }
        }

        {
            let title = if ticket.subject.trim().is_empty() {
                t!("support.issue_title_fallback", id = ticket.id.as_str()).to_string()
            } else {
                ticket.subject.trim().to_string()
            };
            let body = if matches!(kind, SupportMenuKind::ConversationHeader) {
                ticket
                    .messages
                    .iter()
                    .rev()
                    .find(|m| m.is_customer())
                    .map(|m| m.text.clone())
                    .unwrap_or_default()
            } else {
                ticket.last_preview.clone()
            };
            items.push(
                PopoverMenuItem::new("menu-convert", t!("support.menu.convert").to_string())
                    .icon("tag")
                    .on_click({
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_popover_menu(cx);
                                cx.emit(SupportViewEvent::ConvertToIssue {
                                    title: title.clone(),
                                    body: body.clone(),
                                });
                            });
                        }
                    }),
            );
        }

        {
            let rid = id.clone();
            items.push(
                PopoverMenuItem::new("menu-resolve", t!("support.menu.resolve").to_string())
                    .icon("circle-check")
                    .on_click({
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_popover_menu(cx);
                                cx.emit(SupportViewEvent::Resolve {
                                    id: rid.clone(),
                                    resolution: "solved".to_string(),
                                });
                            });
                        }
                    }),
            );
        }

        items
    }

    fn render_ticket_popover(
        &self,
        kind: SupportMenuKind,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let items = self.build_ticket_menu_items(&kind, entity.clone());
        PopoverMenu::new(pos, items).on_close({
            let entity = entity.clone();
            move |_, cx| {
                entity.update(cx, |this, cx| {
                    this.close_popover_menu(cx);
                });
            }
        })
    }

    /// Empty conversation pane — shown when no ticket is selected. Friendly
    /// onboarding block instead of a bare "Sélectionnez un ticket" so a
    /// first-time agent knows what the pane is for and how to get started.
    fn render_empty_conversation(&self) -> Div {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .p(px(24.0))
            .child(
                div()
                    .size(px(48.0))
                    .rounded_full()
                    .bg(ShellDeckColors::primary().opacity(0.12))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_size(px(22.0))
                            .text_color(ShellDeckColors::primary())
                            .child("💬"),
                    ),
            )
            .child(
                div()
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("support.empty.tickets").to_string()),
            )
            .child(
                div()
                    .max_w(px(320.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.empty.tickets_hint").to_string()),
            )
    }

    fn render_conversation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ticket) = self.detail.clone() else {
            return self.render_empty_conversation();
        };
        let tid = ticket.id.clone();

        // Header — context card. Big subject, then a single meta row with the
        // contact avatar + name, the status + priority as color-coded Badges,
        // the assignee in plain French, and the "last activity" time. Aim is
        // that a non-tech agent can read the whole context in ~2 seconds.
        let contact_name = ticket.contact.display();
        let assignee = assignee_display(&ticket.assignee, Some(self.my_email()));
        let last_at = ticket.last_at;
        let subject = if ticket.subject.trim().is_empty() {
            t!("support.empty.no_subject").to_string()
        } else {
            ticket.subject.clone()
        };

        let meta_row = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(8.0))
            .child(
                Avatar::new()
                    .name(contact_name.clone())
                    .size(AvatarSize::Xs),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_primary())
                    .child(contact_name),
            )
            .child(status_badge(&ticket.status))
            .child(priority_badge(&ticket.priority))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.assigned_to", name = assignee).to_string()),
            );
        let mut meta_row = meta_row;
        if last_at > 0.0 {
            meta_row = meta_row.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.last_exchange", time = rel_time(last_at)).to_string()),
            );
        }

        let header = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .px(px(16.0))
            .py(px(12.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(px(16.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(subject),
                    )
                    .child({
                        let entity = cx.entity();
                        IconButton::new("ellipsis-vertical")
                            .variant(ButtonVariant::Ghost)
                            .size(gpui::px(28.0))
                            .icon_size(gpui::px(14.0))
                            .on_click({
                                move |event, _, cx| {
                                    entity.update(cx, |this, cx| {
                                        this.popover_menu = Some((
                                            SupportMenuKind::ConversationHeader,
                                            event.position(),
                                        ));
                                        cx.notify();
                                    });
                                }
                            })
                    }),
            )
            .child(meta_row)
            .child(self.render_header_subpanels(&ticket, cx));

        // Messages (scrollable). Subtle background tint so the thread reads
        // as a distinct "conversation surface", separate from the white
        // header + action bar chrome. `bg_surface` is the same token adabraka
        // uses for card bodies — light-mode = warm cream, dark-mode = darker
        // panel, so the contrast stays gentle in both themes. `track_scroll`
        // wires the ScrollHandle that `set_detail` calls `scroll_to_bottom`
        // on, so opening a ticket lands on the newest message.
        let mut messages = div()
            .id("support-messages")
            .flex_1()
            // `min_h_0` on a flex_1 child is what actually lets the pane
            // shrink below its content height and enable overflow_y_scroll;
            // without it the tall content pushes the whole conversation
            // column past the composer.
            .min_h(px(0.0))
            .overflow_y_scroll()
            .track_scroll(&self.messages_scroll)
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(14.0))
            .pt(px(14.0))
            // Extra bottom padding so scroll_to_bottom leaves visible air
            // between the last bubble and the action bar's top border.
            .pb(px(20.0))
            .bg(ShellDeckColors::bg_surface());
        if ticket.messages.is_empty() {
            messages = messages.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.empty.messages").to_string()),
            );
        } else {
            for m in &ticket.messages {
                messages = messages.child(Self::render_message(m, &self.me));
            }
        }

        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            // Without min_h(0) on the flex_col, the flex_1 messages pane
            // below can't correctly compute its "remaining height" — tall
            // conversations then stack past the composer instead of
            // scrolling internally, and the last bubble ends up crushed
            // against the action bar. Same idiom as parent uses at line
            // 1762.
            .min_h(px(0.0))
            .child(header)
            .child(messages)
            .child(self.render_composer(&tid, cx))
    }

    /// Priority / assignee pickers opened from the header kebab menu.
    fn render_header_subpanels(
        &self,
        ticket: &SupportTicket,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if !self.priority_menu_open && !self.assign_menu_open {
            return div().into_any_element();
        }

        let id = ticket.id.clone();
        let mut panel = div().flex().flex_col().gap(px(6.0)).pt(px(4.0));

        if self.priority_menu_open {
            let mut prio_row = div()
                .w_full()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.0));
            for p in ["low", "normal", "high", "urgent"] {
                let pid = id.clone();
                let active = ticket.priority == p;
                let mut chip = div()
                    .id(ElementId::from(SharedString::from(format!(
                        "sup-pchip-{p}"
                    ))))
                    .p(px(2.0))
                    .rounded_full()
                    .cursor_pointer()
                    .border_2()
                    .child(priority_badge(p))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.priority_menu_open = false;
                        cx.emit(SupportViewEvent::SetPriority {
                            id: pid.clone(),
                            priority: p.to_string(),
                        });
                    }));
                if active {
                    chip = chip.border_color(ShellDeckColors::primary());
                } else {
                    chip = chip.border_color(gpui::transparent_black()).opacity(0.55);
                }
                prio_row = prio_row.child(chip);
            }
            panel = panel.child(prio_row);
        }

        if self.assign_menu_open {
            let mut list = div()
                .id("sup-assign-list")
                .w_full()
                .max_h(px(160.0))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(2.0));
            {
                let uid = id.clone();
                list = list.child(self.action_button(
                    "sup-unassign",
                    "— Non attribué —".to_string(),
                    Some("user"),
                    cx,
                    move |this, cx| {
                        this.assign_menu_open = false;
                        cx.emit(SupportViewEvent::Assign {
                            id: uid.clone(),
                            assignee: String::new(),
                        });
                    },
                ));
            }
            for agent in &self.agents {
                let aid = id.clone();
                let email = agent.email.clone();
                let display_name = if agent.name.trim().is_empty() {
                    agent.email.clone()
                } else {
                    agent.name.clone()
                };
                let email_below = if agent.name.trim().is_empty() {
                    String::new()
                } else {
                    agent.email.clone()
                };
                let mut row = div()
                    .id(ElementId::from(SharedString::from(format!(
                        "sup-ag-{}",
                        agent.email
                    ))))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(9.0))
                    .py(px(5.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                    .child(
                        Avatar::new()
                            .name(display_name.clone())
                            .size(AvatarSize::Xs),
                    );
                let mut name_col = div().flex().flex_col().child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ShellDeckColors::text_primary())
                        .child(display_name),
                );
                if !email_below.is_empty() {
                    name_col = name_col.child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(email_below),
                    );
                }
                row = row.child(name_col).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.assign_menu_open = false;
                        cx.emit(SupportViewEvent::Assign {
                            id: aid.clone(),
                            assignee: email.clone(),
                        });
                    },
                ));
                list = list.child(row);
            }
            panel = panel.child(list);
        }

        panel.into_any_element()
    }

    fn render_composer(&self, _ticket_id: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let is_note = self.compose_note;
        let toggle =
            |label: &str, icon: &'static str, active: bool, note: bool, cx: &mut Context<Self>| {
                let color = if active {
                    ShellDeckColors::text_primary()
                } else {
                    ShellDeckColors::text_muted()
                };
                let mut b = div()
                    .id(ElementId::from(SharedString::from(format!(
                        "compose-mode-{note}"
                    ))))
                    .px(px(8.0))
                    .py(px(3.0))
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(lucide_icon(icon, 11.0, color))
                    .child(label.to_string())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.compose_note = note;
                        cx.notify();
                    }));
                if active {
                    b = b.bg(ShellDeckColors::selected_bg()).text_color(color);
                } else {
                    b = b.text_color(color);
                }
                b
            };

        let placeholder = if is_note {
            t!("support.note_placeholder").to_string()
        } else {
            t!("support.compose.reply_placeholder").to_string()
        };

        let reply_label = t!("support.compose.reply").to_string();
        let note_label = t!("support.note_internal").to_string();

        // 2-row composer: (1) mode toggle Réponse / Note interne (small
        // chips), (2) the Input widget flex_1 with the send button pinned
        // to its right so the reply flow reads as a single line. Previously
        // the send button sat on its own row below the Input, adding an
        // otherwise pointless third row of chrome.
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .px(px(14.0))
            .py(px(10.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(toggle(&reply_label, "reply", !is_note, false, cx))
                    .child(toggle(&note_label, "sticky-note", is_note, true, cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div().flex_1().child(
                            Input::new(&self.composer_state)
                                .size(InputSize::Md)
                                .placeholder(placeholder)
                                .on_enter({
                                    let entity = cx.entity();
                                    move |_v, cx| {
                                        entity.update(cx, |this, cx| this.send_composer(cx));
                                    }
                                }),
                        ),
                    )
                    .child(
                        div()
                            .id("support-send")
                            .flex_shrink_0()
                            .px(px(14.0))
                            .py(px(7.0))
                            .rounded(px(6.0))
                            .bg(if is_note {
                                ShellDeckColors::warning()
                            } else {
                                ShellDeckColors::primary()
                            })
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white())
                            .cursor_pointer()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .child(lucide_icon("send", 13.0, white()))
                            .child(if is_note {
                                t!("support.compose.add_note").to_string()
                            } else {
                                t!("support.send").to_string()
                            })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.send_composer(cx);
                            })),
                    ),
            )
    }

    fn render_section_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = |label: String,
                   icon: &'static str,
                   section: SupportSection,
                   cx: &mut Context<Self>| {
            let active = self.section == section;
            let entity = cx.entity();
            Self::compact_filter_button(
                ElementId::from(SharedString::from(format!("sup-sec-{section:?}"))),
                label,
            )
            .variant(if active {
                ButtonVariant::Default
            } else {
                ButtonVariant::Outline
            })
            .icon(IconSource::from(icon))
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| {
                    this.section = section;
                    if section == SupportSection::Requests {
                        cx.emit(SupportViewEvent::IssuesRefresh);
                    }
                    cx.notify();
                });
            })
        };
        div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(10.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(tab(
                t!("support.tickets").to_string(),
                "inbox",
                SupportSection::Tickets,
                cx,
            ))
            .child(tab(
                t!("support.requests_count", count = self.issues.len()).to_string(),
                "tag",
                SupportSection::Requests,
                cx,
            ))
    }

    fn render_requests(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(12.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(lucide_icon("tag", 14.0, ShellDeckColors::primary()))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("support.requests").to_string()),
                    ),
            )
            .child(
                IconButton::new("refresh")
                    .variant(ButtonVariant::Ghost)
                    .size(gpui::px(28.0))
                    .icon_size(gpui::px(12.0))
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(SupportViewEvent::IssuesRefresh);
                    })),
            );

        let mut list = div()
            .id("sup-issues-list")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        if self.issues.is_empty() {
            list = list.child(
                div()
                    .p(px(16.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.empty.requests").to_string()),
            );
        } else {
            for iss in &self.issues {
                list = list.child(self.render_issue_row(iss, cx));
            }
        }

        let left = div()
            .w(px(340.0))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .child(header)
            .child(list);

        div()
            .flex_1()
            .flex()
            .min_h(px(0.0))
            .child(left)
            .child(self.render_issue_detail(cx))
    }

    fn render_issue_row(&self, iss: &Issue, cx: &mut Context<Self>) -> impl IntoElement {
        let id = iss.id.clone();
        let selected = self.issue_selected.as_deref() == Some(iss.id.as_str());
        let title = if iss.title.trim().is_empty() {
            t!("support.issue.no_title").to_string()
        } else {
            iss.title.clone()
        };
        let when = rel_time(iss.updated_at);

        let mut row = div()
            .id(ElementId::from(SharedString::from(format!("iss-{}", iss.id))))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px(px(10.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .cursor_pointer()
            .hover(|s| s.bg(ShellDeckColors::hover_bg()))
            .on_click(cx.listener(move |_t, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                cx.emit(SupportViewEvent::SelectIssue(id.clone()));
            }));
        if selected {
            row = row.bg(ShellDeckColors::selected_bg());
        }

        let mut meta = format!("{} · {}", iss.tenant_name, iss.source);
        if iss.comment_count > 0 {
            meta.push_str(&format!(
                " · {}",
                t!("support.meta.comments", count = iss.comment_count)
            ));
        }
        if let Some(g) = &iss.github {
            meta.push_str(&format!(" · GH #{}", g.number));
        }

        row = row
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(lucide_icon("tag", 12.0, ShellDeckColors::text_muted()))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(13.0))
                            .font_weight(if selected {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::MEDIUM
                            })
                            .text_color(ShellDeckColors::text_primary())
                            .child(title),
                    )
                    .child(issue_status_badge(&iss.status))
                    .child(priority_badge(&iss.priority))
                    .when(!when.is_empty(), |el| {
                        el.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(when),
                        )
                    }),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(meta),
            );
        row
    }

    fn render_empty_issue_detail(&self) -> Div {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .p(px(24.0))
            .child(
                div()
                    .size(px(48.0))
                    .rounded_full()
                    .bg(ShellDeckColors::primary().opacity(0.12))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(lucide_icon("tag", 22.0, ShellDeckColors::primary())),
            )
            .child(
                div()
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("support.empty.requests_open").to_string()),
            )
            .child(
                div()
                    .max_w(px(320.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.empty.requests_hint").to_string()),
            )
    }

    fn render_issue_comment(c: &shelldeck_core::config::issues::IssueComment) -> impl IntoElement {
        let (bg, label, icon) = if c.is_note() {
            (
                ShellDeckColors::warning().opacity(0.12),
                if c.kind.is_empty() {
                    t!("support.issue.system").to_string()
                } else {
                    c.kind.clone()
                },
                "info",
            )
        } else {
            (
                ShellDeckColors::primary().opacity(0.12),
                if c.author.trim().is_empty() {
                    t!("support.issue.comment").to_string()
                } else {
                    c.author.clone()
                },
                "reply",
            )
        };
        div()
            .max_w(px(560.0))
            .rounded(px(8.0))
            .bg(bg)
            .border_1()
            .border_color(ShellDeckColors::border())
            .px(px(10.0))
            .py(px(7.0))
            .flex()
            .flex_col()
            .gap(px(3.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(10.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(lucide_icon(icon, 11.0, ShellDeckColors::text_muted()))
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(label),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(rel_time(c.at)),
                    ),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(c.body.clone()),
            )
    }

    fn close_issue_popover_menu(&mut self, cx: &mut Context<Self>) {
        self.issue_popover_menu = None;
        cx.notify();
    }

    fn build_issue_menu_items(&self, iss: &Issue, entity: Entity<SupportView>) -> Vec<PopoverMenuItem> {
        if !self.issues_staff {
            return vec![];
        }
        let id = iss.id.clone();
        let mut items = Vec::new();

        items.push(
            PopoverMenuItem::new("iss-menu-status", t!("support.menu.status").to_string())
                .icon("filter")
                .on_click({
                    let entity = entity.clone();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.close_issue_popover_menu(cx);
                            this.issue_status_menu = true;
                            this.issue_priority_menu_open = false;
                            this.issue_dispatch_menu = false;
                            cx.notify();
                        });
                    }
                }),
        );
        items.push(
            PopoverMenuItem::new("iss-menu-priority", t!("support.menu.priority").to_string())
                .icon("flag")
                .on_click({
                    let entity = entity.clone();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.close_issue_popover_menu(cx);
                            this.issue_priority_menu_open = true;
                            this.issue_status_menu = false;
                            this.issue_dispatch_menu = false;
                            cx.notify();
                        });
                    }
                }),
        );

        let aid = id.clone();
        items.push(
            PopoverMenuItem::new("iss-menu-assign", t!("support.menu.assign_me").to_string())
                .icon("user-check")
                .on_click({
                    let entity = entity.clone();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.close_issue_popover_menu(cx);
                            cx.emit(SupportViewEvent::IssueAssign {
                                id: aid.clone(),
                                assignee: "me".to_string(),
                            });
                        });
                    }
                }),
        );

        if !self.issue_instances.is_empty() {
            items.push(
                PopoverMenuItem::new("iss-menu-dispatch", t!("support.menu.dispatch").to_string())
                    .icon("server")
                    .on_click({
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_issue_popover_menu(cx);
                                this.issue_dispatch_menu = true;
                                this.issue_status_menu = false;
                                this.issue_priority_menu_open = false;
                                cx.notify();
                            });
                        }
                    }),
            );
        }

        let gid = id.clone();
        if iss.github.is_some() {
            items.push(
                PopoverMenuItem::new("iss-menu-gh", t!("support.menu.github_sync").to_string())
                    .icon("refresh-cw")
                    .on_click({
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_issue_popover_menu(cx);
                                cx.emit(SupportViewEvent::IssueGithubRefresh(gid.clone()));
                            });
                        }
                    }),
            );
        } else {
            items.push(
                PopoverMenuItem::new("iss-menu-gh-push", t!("support.menu.github_create").to_string())
                    .icon("upload")
                    .on_click({
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_issue_popover_menu(cx);
                                cx.emit(SupportViewEvent::IssueGithubPush(gid.clone()));
                            });
                        }
                    }),
            );
        }

        items
    }

    fn render_issue_popover(
        &self,
        iss: &Issue,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let items = self.build_issue_menu_items(iss, entity.clone());
        PopoverMenu::new(pos, items).on_close({
            let entity = entity.clone();
            move |_, cx| {
                entity.update(cx, |this, cx| {
                    this.close_issue_popover_menu(cx);
                });
            }
        })
    }

    fn render_issue_header_subpanels(
        &self,
        iss: &Issue,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if !self.issues_staff {
            return div().into_any_element();
        }
        if !self.issue_status_menu && !self.issue_priority_menu_open && !self.issue_dispatch_menu {
            return div().into_any_element();
        }

        let id = iss.id.clone();
        let mut panel = div().flex().flex_col().gap(px(6.0)).pt(px(4.0));

        if self.issue_status_menu {
            let mut row = div().flex().flex_wrap().items_center().gap(px(6.0));
            for s in [
                "open",
                "triaging",
                "in_progress",
                "blocked",
                "done",
                "closed",
            ] {
                let sid = id.clone();
                let active = iss.status == s;
                let mut chip = div()
                    .id(ElementId::from(SharedString::from(format!(
                        "iss-schip-{s}"
                    ))))
                    .p(px(2.0))
                    .rounded_full()
                    .cursor_pointer()
                    .border_2()
                    .child(issue_status_badge(s))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.issue_status_menu = false;
                        cx.emit(SupportViewEvent::IssueStatus {
                            id: sid.clone(),
                            status: s.to_string(),
                        });
                    }));
                if active {
                    chip = chip.border_color(ShellDeckColors::primary());
                } else {
                    chip = chip.border_color(gpui::transparent_black()).opacity(0.55);
                }
                row = row.child(chip);
            }
            panel = panel.child(row);
        }

        if self.issue_priority_menu_open {
            let mut row = div().flex().flex_wrap().items_center().gap(px(6.0));
            for p in ["low", "normal", "high", "urgent"] {
                let pid = id.clone();
                let active = iss.priority == p;
                let mut chip = div()
                    .id(ElementId::from(SharedString::from(format!(
                        "iss-pchip-{p}"
                    ))))
                    .p(px(2.0))
                    .rounded_full()
                    .cursor_pointer()
                    .border_2()
                    .child(priority_badge(p))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.issue_priority_menu_open = false;
                        cx.emit(SupportViewEvent::IssuePriority {
                            id: pid.clone(),
                            priority: p.to_string(),
                        });
                    }));
                if active {
                    chip = chip.border_color(ShellDeckColors::primary());
                } else {
                    chip = chip.border_color(gpui::transparent_black()).opacity(0.55);
                }
                row = row.child(chip);
            }
            panel = panel.child(row);
        }

        if self.issue_dispatch_menu {
            let mut list = div()
                .id("iss-dispatch-list")
                .w_full()
                .max_h(px(160.0))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(2.0));
            for inst in &self.issue_instances {
                let did = id.clone();
                let iid = inst.id.clone();
                list = list.child(self.action_button(
                    "iss-disp-inst",
                    format!("{} ({})", inst.name, inst.status),
                    Some("server"),
                    cx,
                    move |this, cx| {
                        this.issue_dispatch_menu = false;
                        cx.emit(SupportViewEvent::IssueDispatch {
                            id: did.clone(),
                            instance_id: iid.clone(),
                        });
                    },
                ));
            }
            panel = panel.child(list);
        }

        panel.into_any_element()
    }

    fn render_issue_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(iss) = self.issue_detail.clone() else {
            return self.render_empty_issue_detail().into_any_element();
        };

        let assignee = assignee_display(&iss.assignee, None);
        let mut meta_row = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(8.0))
            .child(issue_status_badge(&iss.status))
            .child(priority_badge(&iss.priority))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.assigned_to", name = assignee).to_string()),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(iss.tenant_name.clone()),
            );
        if let Some(label) = iss
            .site_label
            .as_ref()
            .filter(|l| !l.trim().is_empty())
        {
            meta_row = meta_row.child(
                Badge::new(label.clone()).variant(BadgeVariant::Outline),
            );
        }
        if let Some(g) = &iss.github {
            meta_row = meta_row.child(
                Badge::new(format!("GitHub #{}", g.number)).variant(BadgeVariant::Secondary),
            );
        }
        if iss.updated_at > 0.0 {
            meta_row = meta_row.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!("· mis à jour {}", rel_time(iss.updated_at))),
            );
        }

        let header = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .px(px(16.0))
            .py(px(12.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(8.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(px(16.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(iss.title.clone()),
                    )
                    .when(self.issues_staff, |el| {
                        el.child({
                            let entity = cx.entity();
                            IconButton::new("ellipsis-vertical")
                                .variant(ButtonVariant::Ghost)
                                .size(gpui::px(28.0))
                                .icon_size(gpui::px(14.0))
                                .on_click({
                                    move |event, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.issue_popover_menu = Some(event.position());
                                            cx.notify();
                                        });
                                    }
                                })
                        })
                    }),
            )
            .child(meta_row)
            .child(self.render_issue_header_subpanels(&iss, cx));

        let mut thread = div()
            .id("sup-issue-thread")
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .track_scroll(&self.issues_scroll)
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(14.0))
            .pt(px(14.0))
            .pb(px(20.0))
            .bg(ShellDeckColors::bg_surface());

        if !iss.body.trim().is_empty() {
            thread = thread.child(
                div()
                    .max_w(px(560.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .px(px(10.0))
                    .py(px(7.0))
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .child(lucide_icon(
                                        "sticky-note",
                                        11.0,
                                        ShellDeckColors::text_muted(),
                                    ))
                                    .child(
                                        div()
                                            .text_size(px(10.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(ShellDeckColors::text_muted())
                                            .child(if iss.requested_by.trim().is_empty() {
                                                t!("support.issue.description").to_string()
                                            } else {
                                                iss.requested_by.clone()
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(rel_time(iss.created_at)),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(iss.body.clone()),
                    ),
            );
        } else if iss.comments.is_empty() {
            thread = thread.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.empty.comments").to_string()),
            );
        }
        for c in &iss.comments {
            thread = thread.child(Self::render_issue_comment(c));
        }

        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .child(header)
            .child(thread)
            .child(self.render_issue_composer(cx))
            .into_any_element()
    }

    fn render_issue_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    Input::new(&self.composer_state)
                        .size(InputSize::Sm)
                        .placeholder(t!("support.issue_comment_placeholder").to_string())
                        .prefix(lucide_icon(
                            "reply",
                            14.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .on_enter({
                            let entity = entity.clone();
                            move |_v, cx| {
                                entity.update(cx, |this, cx| this.send_composer(cx));
                            }
                        }),
                ),
            )
            .child(
                Button::new("sup-issue-send", t!("support.send").to_string())
                    .size(ButtonSize::Sm)
                    .h(gpui::px(32.0))
                    .icon(IconSource::from("send"))
                    .on_click({
                        move |_, _, cx| {
                            entity.update(cx, |this, cx| this.send_composer(cx));
                        }
                    }),
            )
    }
}

impl Render for SupportView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search_query(cx);
        let filtered: Vec<SupportTicket> = self
            .tickets
            .iter()
            .filter(|t| self.passes_filter(t) && self.passes_advanced(t, &query))
            .cloned()
            .collect();

        // Left column: header (title + refresh) + filters + list.
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(12.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("support.title").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(if self.loading {
                                t!("support.loading").to_string()
                            } else {
                                t!("support.ticket_count", count = self.counts.all).to_string()
                            }),
                    ),
            )
            .child(
                div()
                    .id("support-refresh")
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                    .child(lucide_icon(
                        "refresh-cw",
                        12.0,
                        ShellDeckColors::text_muted(),
                    ))
                    .child(t!("support.refresh").to_string())
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(SupportViewEvent::Refresh);
                    })),
            );

        let mut list = div()
            .id("support-ticket-list")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col();
        if filtered.is_empty() {
            list = list.child(
                div()
                    .p(px(16.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(if self.has_list_constraints(cx) {
                        t!("support.empty.tickets_filtered").to_string()
                    } else {
                        t!("support.empty.tickets_view").to_string()
                    }),
            );
        } else {
            for t in &filtered {
                list = list.child(self.render_ticket_row(t, cx));
            }
        }

        let mut left = div()
            .w(px(340.0))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .child(header);
        if self.jean_available {
            left = left.child(self.render_jean_strip(cx));
        }
        left = left.child(self.render_filters(cx)).child(list);

        let content = match self.section {
            SupportSection::Tickets => div()
                .flex_1()
                .flex()
                .min_h(px(0.0))
                .child(left)
                .child(self.render_conversation(cx))
                .into_any_element(),
            SupportSection::Requests => self.render_requests(cx).into_any_element(),
        };

        let mut root = div()
            .relative()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(ShellDeckColors::bg_primary())
            .child(self.render_section_tabs(cx))
            .child(content);

        if self.filter_modal_open && self.section == SupportSection::Tickets {
            root = root.child(self.render_filter_modal(cx));
        }

        if let Some((kind, pos)) = self.popover_menu.clone() {
            root = root.child(self.render_ticket_popover(kind, pos, cx));
        }

        if self.section == SupportSection::Requests {
            if let (Some(pos), Some(iss)) = (self.issue_popover_menu, self.issue_detail.clone()) {
                root = root.child(self.render_issue_popover(&iss, pos, cx));
            }
        }

        if let Some(err) = &self.error {
            root = root.child(
                div()
                    .absolute()
                    .bottom(px(12.0))
                    .left(px(12.0))
                    .px(px(12.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::error())
                    .text_size(px(12.0))
                    .text_color(white())
                    .child(err.clone()),
            );
        }

        root
    }
}

/// Human-facing label for a support ticket's status.
pub(crate) fn status_label(s: &str) -> String {
    match s {
        "open" => t!("support.status.open").to_string(),
        "pending" => t!("support.status.pending").to_string(),
        "closed" => t!("support.status.closed").to_string(),
        other => other.to_string(),
    }
}

/// Support ticket status rendered as a color-coded adabraka `Badge`.
/// `open` = Default (primary, "à faire"), `pending` = Warning (waiting on
/// the customer), `closed` = Outline (calm, done).
pub(crate) fn status_badge(s: &str) -> Badge {
    let variant = match s {
        "open" => BadgeVariant::Default,
        "pending" => BadgeVariant::Warning,
        "closed" => BadgeVariant::Outline,
        _ => BadgeVariant::Secondary,
    };
    Badge::new(status_label(s)).variant(variant)
}

pub(crate) fn priority_label(p: &str) -> String {
    match p {
        "low" => t!("support.priority.low").to_string(),
        "normal" => t!("support.priority.normal").to_string(),
        "high" => t!("support.priority.high").to_string(),
        "urgent" => t!("support.priority.urgent").to_string(),
        other => other.to_string(),
    }
}

/// Priority level as an adabraka `Badge` with a color that matches the
/// severity: low → Outline (neutral), normal → Secondary (grey), high →
/// Warning (orange), urgent → Destructive (red). Used everywhere a
/// priority is displayed to a reader.
pub(crate) fn priority_badge(p: &str) -> Badge {
    let variant = match p {
        "urgent" => BadgeVariant::Destructive,
        "high" => BadgeVariant::Warning,
        "low" => BadgeVariant::Outline,
        _ => BadgeVariant::Secondary,
    };
    Badge::new(priority_label(p)).variant(variant)
}

pub(crate) fn issue_status_label(s: &str) -> String {
    match s {
        "open" => t!("support.issue_status.open").to_string(),
        "triaging" => t!("support.issue_status.triaging").to_string(),
        "in_progress" => t!("support.issue_status.in_progress").to_string(),
        "blocked" => t!("support.issue_status.blocked").to_string(),
        "done" => t!("support.issue_status.done").to_string(),
        "closed" => t!("support.issue_status.closed").to_string(),
        other => other.to_string(),
    }
}

/// Issue status rendered as a color-coded adabraka `Badge`, mirroring the
/// severity/state mapping used across the app: `open` / `in_progress` are
/// primary (active work), `triaging` is neutral grey, `blocked` is
/// destructive (something's stuck), `done` / `closed` are outline (calm).
pub(crate) fn issue_status_badge(s: &str) -> Badge {
    let variant = match s {
        "open" | "in_progress" => BadgeVariant::Default,
        "triaging" => BadgeVariant::Secondary,
        "blocked" => BadgeVariant::Destructive,
        "done" | "closed" => BadgeVariant::Outline,
        _ => BadgeVariant::Secondary,
    };
    Badge::new(issue_status_label(s)).variant(variant)
}

/// Human-friendly assignee label: `me` / empty → unassigned; email
/// stays as email; a full-name assignee stays as-is.
pub(crate) fn assignee_display(assignee: &str, self_email: Option<&str>) -> String {
    let a = assignee.trim();
    if a.is_empty() {
        return t!("support.assignee.none").to_string();
    }
    if a.eq_ignore_ascii_case("me") {
        return t!("support.assignee.me").to_string();
    }
    if let Some(me) = self_email {
        if a.eq_ignore_ascii_case(me) {
            return t!("support.assignee.me").to_string();
        }
    }
    a.to_string()
}

/// The next priority in a low→normal→high→urgent→low cycle.
fn next_priority(p: &str) -> &'static str {
    match p {
        "low" => "normal",
        "normal" => "high",
        "high" => "urgent",
        _ => "low",
    }
}

/// Rough relative time from an epoch-ms timestamp.
fn rel_time(at_ms: f64) -> String {
    crate::i18n::rel_time(at_ms)
}
