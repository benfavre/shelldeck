//! Support mode — a native two-pane helpdesk console over the token-gated
//! support API. Left: view filters + ticket list. Right: the conversation, a
//! reply/note composer, and an action bar (status / priority / assign / resolve).
//!
//! The view holds data and captures composer text; all network happens in the
//! `Workspace` (background executor) driven by [`SupportViewEvent`].

use crate::icons::lucide_icon;
use crate::scale::px;
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
    fn label(self) -> &'static str {
        match self {
            SupportFilter::All => "Tous",
            SupportFilter::Unassigned => "Non attribués",
            SupportFilter::Mine => "Les miens",
            SupportFilter::Open => "Ouverts",
            SupportFilter::Pending => "En attente",
            SupportFilter::Breaching => "SLA",
            SupportFilter::Closed => "Résolus",
        }
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
    /// Real `Input` state backing the reply / internal-note composer.
    composer_state: Entity<InputState>,
    compose_note: bool,
    loading: bool,
    error: Option<String>,
    assign_menu_open: bool,
    priority_menu_open: bool,
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
    focus_handle: FocusHandle,
    /// Scroll handle for the messages pane. `set_detail` calls
    /// `scroll_to_bottom()` on it so opening a ticket lands the reader on
    /// the latest message (the classic chat/messaging behavior), not on
    /// the top of the history.
    messages_scroll: ScrollHandle,
}

impl SupportView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            tickets: Vec::new(),
            counts: SupportCounts::default(),
            me: SupportMe::default(),
            agents: Vec::new(),
            selected_id: None,
            detail: None,
            filter: SupportFilter::All,
            composer_state: cx.new(InputState::new),
            compose_note: false,
            loading: false,
            error: None,
            assign_menu_open: false,
            priority_menu_open: false,
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
                            .child("JEANCLAUDE"),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!("{} actifs", self.jean_active)),
                    ),
            );

        if self.jean_pending.is_empty() {
            strip = strip.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("Aucune confirmation en attente"),
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

    fn render_filters(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div()
            .flex()
            .flex_wrap()
            .gap(px(4.0))
            .px(px(10.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border());
        for f in SupportFilter::ALL {
            let active = self.filter == f;
            let count = f.count(&self.counts);
            let mut chip = div()
                .id(ElementId::from(SharedString::from(format!(
                    "sf-{}",
                    f.label()
                ))))
                .flex()
                .items_center()
                .gap(px(4.0))
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(px(12.0))
                .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                .child(f.label().to_string())
                .child(
                    div()
                        .px(px(5.0))
                        .rounded(px(8.0))
                        .bg(ShellDeckColors::badge_bg())
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(count.to_string()),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.filter = f;
                    cx.notify();
                }));
            if active {
                chip = chip
                    .bg(ShellDeckColors::selected_bg())
                    .text_color(ShellDeckColors::text_primary());
            } else {
                chip = chip.text_color(ShellDeckColors::text_muted());
            }
            row = row.child(chip);
        }
        row
    }

    fn render_ticket_row(&self, t: &SupportTicket, cx: &mut Context<Self>) -> impl IntoElement {
        let id = t.id.clone();
        let selected = self.selected_id.as_deref() == Some(t.id.as_str());
        let subject = if t.subject.trim().is_empty() {
            "(sans objet)".to_string()
        } else {
            t.subject.clone()
        };

        let mut row = div()
            .id(ElementId::from(SharedString::from(format!("tk-{}", t.id))))
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px(px(10.0))
            .py(px(8.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .cursor_pointer()
            .hover(|s| s.bg(ShellDeckColors::hover_bg()))
            .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                cx.emit(SupportViewEvent::SelectTicket(id.clone()));
            }));
        if selected {
            row = row.bg(ShellDeckColors::selected_bg());
        }

        // Line 1: channel glyph + subject + priority dot + time
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
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t.channel_glyph()),
                )
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
                ),
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
                "Note interne",
            )
        } else if msg.is_customer() {
            (ShellDeckColors::bg_surface(), false, "Client")
        } else {
            (ShellDeckColors::primary().opacity(0.12), true, "Agent")
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
            .unwrap_or_else(|| label.to_string());

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
                    .child("Aucun ticket ouvert"),
            )
            .child(
                div()
                    .max_w(px(320.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        "Choisis un ticket dans la liste à gauche pour lire l'échange, \
                         y répondre, changer le statut ou l'attribuer à un collègue.",
                    ),
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
            "(sans objet)".to_string()
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
                    .child(format!("Assigné à {assignee}")),
            );
        let mut meta_row = meta_row;
        if last_at > 0.0 {
            meta_row = meta_row.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!("· dernier échange {}", rel_time(last_at))),
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
                    .text_size(px(16.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(subject),
            )
            .child(meta_row);

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
                    .child("Aucun message"),
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
            .child(self.render_action_bar(&ticket, cx))
            .child(self.render_composer(&tid, cx))
    }

    fn render_action_bar(
        &self,
        ticket: &SupportTicket,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = ticket.id.clone();
        let is_pending = ticket.status == "pending";
        let is_mine =
            !self.my_email().is_empty() && ticket.assignee.eq_ignore_ascii_case(self.my_email());
        let (status_next, status_label_next) = if is_pending {
            ("open".to_string(), "Rouvrir".to_string())
        } else {
            // Short label so the whole action bar fits on one row on a
            // typical sheet width — the meaning "put on hold, waiting on
            // the customer" is carried by the status Badge already visible
            // in the header ("en attente client").
            ("pending".to_string(), "En attente".to_string())
        };

        // Single flex_wrap row of buttons. Order = "meta" first (status,
        // assign, priority) → "workflow" last (Jean, convert, résoudre).
        // The green "Résoudre" sits at the tail; when the row wraps it lands
        // at the end of the last visual line — no positional gymnastics with
        // flex_1 spacers that produced overlaps against the composer below.
        let mut actions = div().flex().flex_wrap().items_center().gap(px(6.0));

        // Status toggle — the most common state change.
        {
            let sid = id.clone();
            let snext = status_next.clone();
            actions = actions.child(self.action_button(
                "sup-status",
                status_label_next,
                Some(if is_pending { "circle-check" } else { "clock" }),
                cx,
                move |_this, cx| {
                    cx.emit(SupportViewEvent::SetStatus {
                        id: sid.clone(),
                        status: snext.clone(),
                    });
                },
            ));
        }
        // "M'attribuer" — only when not already assigned to me.
        if !is_mine {
            let aid = id.clone();
            actions = actions.child(self.action_button(
                "sup-assign-me",
                "M'attribuer".to_string(),
                Some("user-check"),
                cx,
                move |_this, cx| {
                    cx.emit(SupportViewEvent::Assign {
                        id: aid.clone(),
                        assignee: "me".to_string(),
                    });
                },
            ));
        }
        // Priority menu toggle — value already visible in the header badge.
        {
            actions = actions.child(self.action_button(
                "sup-priority",
                "Priorité…".to_string(),
                Some("flag"),
                cx,
                move |this, cx| {
                    this.priority_menu_open = !this.priority_menu_open;
                    this.assign_menu_open = false;
                    cx.notify();
                },
            ));
        }
        // Assign menu toggle.
        {
            actions = actions.child(self.action_button(
                "sup-assign",
                "Attribuer…".to_string(),
                Some("users"),
                cx,
                move |this, cx| {
                    this.assign_menu_open = !this.assign_menu_open;
                    this.priority_menu_open = false;
                    cx.notify();
                },
            ));
        }

        // "Jean" — file this ticket through JeanClaude's Slack intake. Short
        // label so the row stays on one line; the tooltip / bot name reads
        // long enough for a non-tech agent to recognize it.
        if self.jean_available {
            actions = actions.child(self.action_button(
                "sup-to-jean",
                "Jean".to_string(),
                Some("send"),
                cx,
                move |this, cx| {
                    if let Some(text) = this.jean_ticket_text() {
                        cx.emit(SupportViewEvent::SendToJean(text));
                    }
                },
            ));
        }

        // "Convertir" — turn this ticket into a tracked request.
        actions = actions.child(self.action_button(
            "sup-to-issue",
            "Convertir".to_string(),
            Some("tag"),
            cx,
            move |this, cx| {
                if let Some(t) = this.detail.as_ref() {
                    let title = if t.subject.trim().is_empty() {
                        format!("Demande support {}", t.id)
                    } else {
                        t.subject.trim().to_string()
                    };
                    let body = t
                        .messages
                        .iter()
                        .rev()
                        .find(|m| m.is_customer())
                        .map(|m| m.text.clone())
                        .unwrap_or_default();
                    cx.emit(SupportViewEvent::ConvertToIssue { title, body });
                }
            },
        ));

        // Primary "Résoudre" — same height as the rest (py 5, no extra
        // vertical space) but green + semibold so the happy-path action
        // still reads as the primary CTA without breaking the row rhythm.
        {
            let rid = id.clone();
            actions = actions.child(
                div()
                    .id("sup-resolve")
                    .px(px(12.0))
                    .py(px(5.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::success())
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(white())
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::success().opacity(0.85)))
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(lucide_icon("circle-check", 12.0, white()))
                    .child("Résoudre")
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                        cx.emit(SupportViewEvent::Resolve {
                            id: rid.clone(),
                            resolution: "solved".to_string(),
                        });
                    })),
            );
        }

        let mut bar = div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .px(px(14.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(actions);

        // Priority picker — colored Badge chips (same visual as the request
        // sheet) instead of plain text buttons. The active priority gets a
        // 2px primary ring; the rest sit at 0.55 opacity so they still
        // read as clickable options.
        if self.priority_menu_open {
            let mut prio_row = div()
                .w_full()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.0))
                .mt(px(2.0));
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
            bar = bar.child(prio_row);
        }

        // Assignee picker popover (inline scrollable row).
        if self.assign_menu_open {
            let mut list = div()
                .id("sup-assign-list")
                .w_full()
                .mt(px(4.0))
                .max_h(px(160.0))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(2.0));
            // Unassign option.
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
                // Row = tiny avatar (auto-initials) + display name over the
                // muted email. Much easier to scan than a raw
                // `"Name <email>"` string when the assignee list gets long.
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
            bar = bar.child(list);
        }

        bar
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
            "Note interne (non envoyée au client)…"
        } else {
            "Votre réponse… (Entrée pour envoyer)"
        };

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
                    .child(toggle("Réponse", "reply", !is_note, false, cx))
                    .child(toggle("Note interne", "sticky-note", is_note, true, cx)),
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
                                "Ajouter la note"
                            } else {
                                "Envoyer"
                            })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.send_composer(cx);
                            })),
                    ),
            )
    }

    fn render_section_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = |label: &str, section: SupportSection, cx: &mut Context<Self>| {
            let active = self.section == section;
            let mut b = div()
                .id(ElementId::from(SharedString::from(format!(
                    "sup-sec-{label}"
                ))))
                .px(px(12.0))
                .py(px(7.0))
                .rounded(px(6.0))
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .child(label.to_string())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.section = section;
                    if section == SupportSection::Requests {
                        cx.emit(SupportViewEvent::IssuesRefresh);
                    }
                    cx.notify();
                }));
            if active {
                b = b
                    .bg(ShellDeckColors::selected_bg())
                    .text_color(ShellDeckColors::text_primary());
            } else {
                b = b.text_color(ShellDeckColors::text_muted());
            }
            b
        };
        div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(10.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(tab("Tickets", SupportSection::Tickets, cx))
            .child(tab(
                &format!("Demandes ({})", self.issues.len()),
                SupportSection::Requests,
                cx,
            ))
    }

    fn render_requests(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // Left: issues list.
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
                    .child("Aucune demande."),
            );
        } else {
            for iss in &self.issues {
                let id = iss.id.clone();
                let selected = self.issue_selected.as_deref() == Some(iss.id.as_str());
                let mut row = div()
                    .id(ElementId::from(SharedString::from(format!(
                        "iss-{}",
                        iss.id
                    ))))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                    .on_click(cx.listener(move |_t, _: &ClickEvent, _, cx| {
                        cx.emit(SupportViewEvent::SelectIssue(id.clone()))
                    }));
                if selected {
                    row = row.bg(ShellDeckColors::selected_bg());
                }
                row = row
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .child(issue_status_badge(&iss.status))
                            .child(
                                div()
                                    .flex_1()
                                    .overflow_hidden()
                                    .whitespace_nowrap()
                                    .text_size(px(13.0))
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(iss.title.clone()),
                            )
                            .child(priority_badge(&iss.priority)),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!(
                                "{} · {} · {} comm.{}",
                                iss.tenant_name,
                                iss.source,
                                iss.comment_count,
                                iss.github
                                    .as_ref()
                                    .map(|g| format!(" · GH #{}", g.number))
                                    .unwrap_or_default()
                            )),
                    );
                list = list.child(row);
            }
        }
        let left = div()
            .w(px(320.0))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .child(list);

        div()
            .flex_1()
            .flex()
            .min_h(px(0.0))
            .child(left)
            .child(self.render_issue_detail(cx))
    }

    fn render_issue_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(iss) = self.issue_detail.clone() else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_muted())
                .child("Sélectionnez une demande")
                .into_any_element();
        };

        let mut header_line = format!(
            "{} · {} · priorité {}",
            iss.tenant_name,
            issue_status_label(&iss.status),
            priority_label(&iss.priority),
        );
        if let Some(g) = &iss.github {
            header_line.push_str(&format!(" · GitHub #{} ({})", g.number, g.state));
        }
        let header = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px(px(14.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(iss.title.clone()),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(header_line),
            );

        // Body + comments.
        let mut thread = div()
            .id("sup-issue-thread")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(14.0));
        if !iss.body.trim().is_empty() {
            thread = thread.child(
                div()
                    .p(px(10.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::bg_surface())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(iss.body.clone()),
            );
        }
        for c in &iss.comments {
            let (bg, label) = if c.is_note() {
                (ShellDeckColors::warning().opacity(0.10), c.kind.clone())
            } else {
                (ShellDeckColors::primary().opacity(0.08), c.author.clone())
            };
            thread = thread.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .p(px(9.0))
                    .rounded(px(8.0))
                    .bg(bg)
                    .child(
                        div()
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(if label.is_empty() {
                                "—".to_string()
                            } else {
                                label
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(c.body.clone()),
                    ),
            );
        }

        let mut col = div()
            .flex_1()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .child(header)
            .child(thread);
        if self.issues_staff {
            col = col.child(self.render_issue_staff_bar(&iss, cx));
        }
        col = col.child(self.render_issue_composer(cx));
        col.into_any_element()
    }

    fn render_issue_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(14.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(
                div().flex_1().child(
                    Input::new(&self.composer_state)
                        .size(InputSize::Sm)
                        .placeholder("Commenter la demande…")
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
                    .id("sup-issue-send")
                    .px(px(12.0))
                    .py(px(7.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::primary())
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white())
                    .cursor_pointer()
                    .child("Envoyer")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.send_composer(cx))),
            )
    }

    fn render_issue_staff_bar(&self, iss: &Issue, cx: &mut Context<Self>) -> impl IntoElement {
        let id = iss.id.clone();
        let mut bar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .px(px(14.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(ShellDeckColors::border());

        // Status menu toggle.
        bar = bar.child(self.action_button(
            "iss-status",
            format!("Statut : {}", issue_status_label(&iss.status)),
            Some("filter"),
            cx,
            move |this, cx| {
                this.issue_status_menu = !this.issue_status_menu;
                this.issue_assign_menu = false;
                this.issue_dispatch_menu = false;
                cx.notify();
            },
        ));
        // Priority quick-cycle button (low→normal→high→urgent→low).
        {
            let pid = id.clone();
            let next = next_priority(&iss.priority).to_string();
            bar = bar.child(self.action_button(
                "iss-prio",
                format!("Priorité : {}", priority_label(&iss.priority)),
                Some("flag"),
                cx,
                move |_this, cx| {
                    cx.emit(SupportViewEvent::IssuePriority {
                        id: pid.clone(),
                        priority: next.clone(),
                    });
                },
            ));
        }
        // Assign to me.
        {
            let aid = id.clone();
            bar = bar.child(self.action_button(
                "iss-assign-me",
                "M'attribuer".to_string(),
                Some("user-check"),
                cx,
                move |_t, cx| {
                    cx.emit(SupportViewEvent::IssueAssign {
                        id: aid.clone(),
                        assignee: "me".to_string(),
                    })
                },
            ));
        }
        // Dispatch menu toggle.
        if !self.issue_instances.is_empty() {
            bar = bar.child(self.action_button(
                "iss-dispatch",
                "Dispatcher…".to_string(),
                Some("server"),
                cx,
                move |this, cx| {
                    this.issue_dispatch_menu = !this.issue_dispatch_menu;
                    this.issue_status_menu = false;
                    cx.notify();
                },
            ));
        }
        // GitHub.
        if iss.github.is_some() {
            let gid = id.clone();
            bar = bar.child(self.action_button(
                "iss-gh-refresh",
                "GitHub".to_string(),
                Some("refresh-cw"),
                cx,
                move |_t, cx| cx.emit(SupportViewEvent::IssueGithubRefresh(gid.clone())),
            ));
        } else {
            let gid = id.clone();
            bar = bar.child(self.action_button(
                "iss-gh-push",
                "Créer sur GitHub".to_string(),
                Some("upload"),
                cx,
                move |_t, cx| cx.emit(SupportViewEvent::IssueGithubPush(gid.clone())),
            ));
        }

        // Status picker popover.
        if self.issue_status_menu {
            let mut row = div().w_full().flex().flex_wrap().gap(px(4.0)).mt(px(4.0));
            for s in [
                "open",
                "triaging",
                "in_progress",
                "blocked",
                "done",
                "closed",
            ] {
                let sid = id.clone();
                row = row.child(self.action_button(
                    match s {
                        "open" => "iss-s-open",
                        "triaging" => "iss-s-tri",
                        "in_progress" => "iss-s-prog",
                        "blocked" => "iss-s-block",
                        "done" => "iss-s-done",
                        _ => "iss-s-closed",
                    },
                    status_label(s).to_string(),
                    None,
                    cx,
                    move |this, cx| {
                        this.issue_status_menu = false;
                        cx.emit(SupportViewEvent::IssueStatus {
                            id: sid.clone(),
                            status: s.to_string(),
                        });
                    },
                ));
            }
            bar = bar.child(row);
        }
        // Dispatch picker popover.
        if self.issue_dispatch_menu {
            let mut row = div().w_full().flex().flex_col().gap(px(2.0)).mt(px(4.0));
            for inst in &self.issue_instances {
                let did = id.clone();
                let iid = inst.id.clone();
                row = row.child(self.action_button(
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
            bar = bar.child(row);
        }
        bar
    }
}

impl Render for SupportView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let filtered: Vec<SupportTicket> = self
            .tickets
            .iter()
            .filter(|t| self.passes_filter(t))
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
                            .child("Support"),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(if self.loading {
                                "chargement…".to_string()
                            } else {
                                format!("{} tickets", self.counts.all)
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
                    .child("Actualiser")
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
                    .child("Aucun ticket dans cette vue"),
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
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(ShellDeckColors::bg_primary())
            .child(self.render_section_tabs(cx))
            .child(content);

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
pub(crate) fn status_label(s: &str) -> &str {
    match s {
        "open" => "à traiter",
        "pending" => "en attente client",
        "closed" => "résolu",
        other => other,
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
    Badge::new(status_label(s).to_string()).variant(variant)
}

pub(crate) fn priority_label(p: &str) -> &str {
    match p {
        "low" => "basse",
        "normal" => "normale",
        "high" => "haute",
        "urgent" => "urgente",
        other => other,
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
    Badge::new(priority_label(p).to_string()).variant(variant)
}

pub(crate) fn issue_status_label(s: &str) -> &str {
    match s {
        "open" => "à traiter",
        "triaging" => "en analyse",
        "in_progress" => "en cours",
        "blocked" => "bloquée",
        "done" => "terminée",
        "closed" => "clôturée",
        other => other,
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
    Badge::new(issue_status_label(s).to_string()).variant(variant)
}

/// Human-friendly assignee label: `me` / empty → "Non attribué"; email
/// stays as email; a full-name assignee stays as-is.
pub(crate) fn assignee_display(assignee: &str, self_email: Option<&str>) -> String {
    let a = assignee.trim();
    if a.is_empty() {
        return "Non attribué".to_string();
    }
    if a.eq_ignore_ascii_case("me") {
        return "Moi".to_string();
    }
    if let Some(me) = self_email {
        if a.eq_ignore_ascii_case(me) {
            return "Moi".to_string();
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
    if at_ms <= 0.0 {
        return String::new();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(at_ms);
    let secs = ((now - at_ms) / 1000.0).max(0.0);
    if secs < 60.0 {
        "à l'instant".to_string()
    } else if secs < 3600.0 {
        format!("il y a {} min", (secs / 60.0) as i64)
    } else if secs < 86400.0 {
        format!("il y a {} h", (secs / 3600.0) as i64)
    } else {
        format!("il y a {} j", (secs / 86400.0) as i64)
    }
}
