//! Support mode — a native two-pane helpdesk console over the token-gated
//! support API. Left: view filters + ticket list. Right: the conversation, a
//! reply/note composer, and an action bar (status / priority / assign / resolve).
//!
//! The view holds data and captures composer text; all network happens in the
//! `Workspace` (background executor) driven by [`SupportViewEvent`].

use gpui::prelude::*;
use gpui::*;
use crate::scale::px;

use shelldeck_core::config::manage_support::{
    SupportAgent, SupportCounts, SupportMe, SupportMessage, SupportTicket,
};

use crate::theme::ShellDeckColors;

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
    Send { id: String, text: String, note: bool },
    SetStatus { id: String, status: String },
    SetPriority { id: String, priority: String },
    Assign { id: String, assignee: String },
    Resolve { id: String, resolution: String },
    /// Confirm/reject a JeanClaude pending ticket from the Support strip.
    JeanConfirm(String),
    JeanReject(String),
    /// File the selected ticket to JeanClaude (the composed text via /api/say).
    SendToJean(String),
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
    composer: String,
    compose_note: bool,
    loading: bool,
    error: Option<String>,
    assign_menu_open: bool,
    priority_menu_open: bool,
    // JeanClaude strip (fed by the workspace when Jean config is present).
    jean_available: bool,
    jean_pending: Vec<(String, String)>,
    jean_active: usize,
    focus_handle: FocusHandle,
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
            composer: String::new(),
            compose_note: false,
            loading: false,
            error: None,
            assign_menu_open: false,
            priority_menu_open: false,
            jean_available: false,
            jean_pending: Vec::new(),
            jean_active: 0,
            focus_handle: cx.focus_handle(),
        }
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
                    *detail = SupportTicket { messages, ..updated };
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
    pub fn set_detail(&mut self, ticket: SupportTicket) {
        // Merge the updated slim ticket into the list too.
        if let Some(existing) = self.tickets.iter_mut().find(|t| t.id == ticket.id) {
            let msgs = existing.messages.clone();
            *existing = SupportTicket {
                messages: msgs,
                ..ticket.clone()
            };
        }
        self.selected_id = Some(ticket.id.clone());
        self.detail = Some(ticket);
        self.composer.clear();
        self.loading = false;
        self.error = None;
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

    fn handle_composer_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match key {
            "enter" => {
                // Enter sends; Shift+Enter inserts a newline (multi-line notes).
                if event.keystroke.modifiers.shift {
                    self.composer.push('\n');
                    cx.notify();
                } else {
                    self.send_composer(cx);
                }
            }
            "backspace" => {
                self.composer.pop();
                cx.notify();
            }
            _ => {
                if let Some(ref kc) = event.keystroke.key_char {
                    if !event.keystroke.modifiers.control && !event.keystroke.modifiers.alt {
                        self.composer.push_str(kc);
                        cx.notify();
                    }
                } else if key.len() == 1
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                {
                    self.composer.push_str(key);
                    cx.notify();
                }
            }
        }
    }

    fn send_composer(&mut self, cx: &mut Context<Self>) {
        let text = self.composer.trim().to_string();
        if text.is_empty() {
            return;
        }
        if let Some(id) = self.selected_id.clone() {
            let note = self.compose_note;
            self.loading = true;
            cx.emit(SupportViewEvent::Send { id, text, note });
            cx.notify();
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
                                .id(ElementId::from(SharedString::from(format!("sj-ok-{thread}"))))
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
                                .id(ElementId::from(SharedString::from(format!("sj-no-{thread}"))))
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
                .id(ElementId::from(SharedString::from(format!("sf-{}", f.label()))))
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
        let prio_color = match t.priority.as_str() {
            "urgent" => ShellDeckColors::error(),
            "high" => ShellDeckColors::warning(),
            _ => ShellDeckColors::text_muted(),
        };
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
                .child(div().size(px(7.0)).rounded_full().bg(prio_color))
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

    fn render_message(msg: &SupportMessage) -> impl IntoElement {
        let (bg, align_end, label) = if msg.is_note() {
            (ShellDeckColors::warning().opacity(0.12), false, "Note interne")
        } else if msg.is_customer() {
            (ShellDeckColors::bg_surface(), false, "Client")
        } else {
            (ShellDeckColors::primary().opacity(0.12), true, "Agent")
        };
        let who = msg
            .name
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| label.to_string());

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
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(rel_time(msg.at)),
                    ),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(msg.text.clone()),
            );

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
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        div()
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
            .hover(|s| s.bg(ShellDeckColors::hover_bg()))
            .child(label)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| on_click(this, cx)))
    }

    fn render_conversation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ticket) = self.detail.clone() else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_muted())
                .child("Sélectionnez un ticket");
        };
        let tid = ticket.id.clone();

        // Header: subject + contact + status/priority.
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
                    .child(if ticket.subject.trim().is_empty() {
                        "(sans objet)".to_string()
                    } else {
                        ticket.subject.clone()
                    }),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!(
                        "{} · {} · priorité {} · {}",
                        ticket.contact.display(),
                        status_label(&ticket.status),
                        priority_label(&ticket.priority),
                        if ticket.is_unassigned() {
                            "non attribué".to_string()
                        } else {
                            ticket.assignee.clone()
                        }
                    )),
            );

        // Messages (scrollable).
        let mut messages = div()
            .id("support-messages")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(14.0));
        if ticket.messages.is_empty() {
            messages = messages.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("Aucun message"),
            );
        } else {
            for m in &ticket.messages {
                messages = messages.child(Self::render_message(m));
            }
        }

        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .child(header)
            .child(messages)
            .child(self.render_action_bar(&ticket, cx))
            .child(self.render_composer(&tid, cx))
    }

    fn render_action_bar(&self, ticket: &SupportTicket, cx: &mut Context<Self>) -> impl IntoElement {
        let id = ticket.id.clone();
        let is_pending = ticket.status == "pending";
        let (status_next, status_label_next) = if is_pending {
            ("open".to_string(), "Rouvrir".to_string())
        } else {
            ("pending".to_string(), "Mettre en attente".to_string())
        };

        let mut bar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(6.0))
            .px(px(14.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(ShellDeckColors::border());

        // Status toggle.
        {
            let sid = id.clone();
            let snext = status_next.clone();
            bar = bar.child(self.action_button(
                "sup-status",
                status_label_next,
                cx,
                move |_this, cx| {
                    cx.emit(SupportViewEvent::SetStatus {
                        id: sid.clone(),
                        status: snext.clone(),
                    });
                },
            ));
        }
        // Assign to me.
        {
            let aid = id.clone();
            bar = bar.child(self.action_button(
                "sup-assign-me",
                "M'attribuer".to_string(),
                cx,
                move |_this, cx| {
                    cx.emit(SupportViewEvent::Assign {
                        id: aid.clone(),
                        assignee: "me".to_string(),
                    });
                },
            ));
        }
        // Priority menu toggle.
        {
            bar = bar.child(self.action_button(
                "sup-priority",
                format!("Priorité : {}", priority_label(&ticket.priority)),
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
            bar = bar.child(self.action_button(
                "sup-assign",
                "Attribuer…".to_string(),
                cx,
                move |this, cx| {
                    this.assign_menu_open = !this.assign_menu_open;
                    this.priority_menu_open = false;
                    cx.notify();
                },
            ));
        }
        // Resolve.
        {
            let rid = id.clone();
            bar = bar.child(
                div()
                    .id("sup-resolve")
                    .px(px(9.0))
                    .py(px(5.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::success())
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white())
                    .cursor_pointer()
                    .child("Résoudre")
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                        cx.emit(SupportViewEvent::Resolve {
                            id: rid.clone(),
                            resolution: "solved".to_string(),
                        });
                    })),
            );
        }

        // "Envoyer à Jean" — file this ticket through JeanClaude's Slack intake.
        if self.jean_available {
            bar = bar.child(self.action_button(
                "sup-to-jean",
                "Envoyer à Jean".to_string(),
                cx,
                move |this, cx| {
                    if let Some(text) = this.jean_ticket_text() {
                        cx.emit(SupportViewEvent::SendToJean(text));
                    }
                },
            ));
        }

        // Priority picker popover (inline row).
        if self.priority_menu_open {
            let mut prio_row = div().w_full().flex().flex_wrap().gap(px(4.0)).mt(px(4.0));
            for p in ["low", "normal", "high", "urgent"] {
                let pid = id.clone();
                prio_row = prio_row.child(self.action_button(
                    match p {
                        "low" => "sup-p-low",
                        "normal" => "sup-p-normal",
                        "high" => "sup-p-high",
                        _ => "sup-p-urgent",
                    },
                    priority_label(p).to_string(),
                    cx,
                    move |this, cx| {
                        this.priority_menu_open = false;
                        cx.emit(SupportViewEvent::SetPriority {
                            id: pid.clone(),
                            priority: p.to_string(),
                        });
                    },
                ));
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
                let label = if agent.name.trim().is_empty() {
                    agent.email.clone()
                } else {
                    format!("{} <{}>", agent.name, agent.email)
                };
                list = list.child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "sup-ag-{}",
                            agent.email
                        ))))
                        .px(px(9.0))
                        .py(px(5.0))
                        .rounded(px(6.0))
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_primary())
                        .cursor_pointer()
                        .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                        .child(label)
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.assign_menu_open = false;
                            cx.emit(SupportViewEvent::Assign {
                                id: aid.clone(),
                                assignee: email.clone(),
                            });
                        })),
                );
            }
            bar = bar.child(list);
        }

        bar
    }

    fn render_composer(&self, _ticket_id: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let is_note = self.compose_note;
        let toggle = |label: &str, active: bool, note: bool, cx: &mut Context<Self>| {
            let mut b = div()
                .id(ElementId::from(SharedString::from(format!("compose-mode-{note}"))))
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .text_size(px(12.0))
                .cursor_pointer()
                .child(label.to_string())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.compose_note = note;
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

        let placeholder = if is_note {
            "Note interne (non envoyée au client)…"
        } else {
            "Votre réponse… (Entrée pour envoyer, Maj+Entrée = nouvelle ligne)"
        };
        let input_display = if self.composer.is_empty() {
            div()
                .text_color(ShellDeckColors::text_muted())
                .child(placeholder.to_string())
        } else {
            div()
                .text_color(ShellDeckColors::text_primary())
                .child(self.composer.clone())
        };

        let composer_border = if is_note {
            ShellDeckColors::warning()
        } else {
            ShellDeckColors::border()
        };

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
                    .child(toggle("Réponse", !is_note, false, cx))
                    .child(toggle("Note interne", is_note, true, cx)),
            )
            .child(
                div()
                    .id("support-composer")
                    .track_focus(&self.focus_handle)
                    .on_key_down(cx.listener(|this, e: &KeyDownEvent, _w, cx| {
                        this.handle_composer_key(e, cx);
                    }))
                    .w_full()
                    .min_h(px(52.0))
                    .px(px(10.0))
                    .py(px(8.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(composer_border)
                    .text_size(px(13.0))
                    .cursor_text()
                    .child(input_display),
            )
            .child(
                div().flex().justify_end().child(
                    div()
                        .id("support-send")
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
                        .child(if is_note { "Ajouter la note" } else { "Envoyer" })
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.send_composer(cx);
                        })),
                ),
            )
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
                    .px(px(8.0))
                    .py(px(4.0))
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                    .child("\u{21BB} Actualiser")
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

        let mut root = div()
            .size_full()
            .flex()
            .bg(ShellDeckColors::bg_primary())
            .child(left)
            .child(self.render_conversation(cx));

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

fn status_label(s: &str) -> &str {
    match s {
        "open" => "ouvert",
        "pending" => "en attente",
        "closed" => "résolu",
        other => other,
    }
}

fn priority_label(p: &str) -> &str {
    match p {
        "low" => "basse",
        "normal" => "normale",
        "high" => "haute",
        "urgent" => "urgente",
        other => other,
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
