//! JeanClaude console (Dev mode) — a native replacement for the bot's web
//! dashboard. Tabs: Aperçu (bot status + say + pending confirmations + active
//! tickets), Historique (filter + list + detail), Cibles (targets CRUD),
//! Mémoire (memory CRUD).
//!
//! The view holds cached data + input buffers and emits [`JeanViewEvent`]; all
//! network is serviced by the `Workspace` on the background executor.

use gpui::prelude::*;
use gpui::*;
use crate::scale::px;

use shelldeck_core::config::jeanclaude::{
    JeanMemory, JeanState, JeanTargets, JeanTicket,
};

use crate::theme::ShellDeckColors;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JeanTab {
    Overview,
    History,
    Targets,
    Memory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    None,
    Say,
    HistorySearch,
    TargetDomain,
    TargetHost,
    TargetNote,
    MemMatch,
    MemText,
}

#[derive(Debug, Clone)]
pub enum JeanViewEvent {
    Refresh,
    SetPaused(bool),
    SetConcurrency(i64),
    Say(String),
    Confirm(String),
    Reject(String),
    Cancel(String),
    Force(String),
    SelectTicket(String),
    LoadHistory { q: String, status: String },
    LoadTargets,
    LoadMemory,
    AddTarget { domain: String, ssh_host: String, note: String },
    RemoveTarget(String),
    AddMemory { kind: String, match_: String, text: String },
    RemoveMemory(String),
}

impl EventEmitter<JeanViewEvent> for JeanView {}

pub struct JeanView {
    tab: JeanTab,
    state: Option<JeanState>,
    history: Vec<JeanTicket>,
    targets: Option<JeanTargets>,
    memory: Vec<JeanMemory>,
    detail: Option<JeanTicket>,
    history_status: String,
    loading: bool,
    error: Option<String>,
    // input buffers
    active_field: Field,
    say_input: String,
    history_search: String,
    t_domain: String,
    t_host: String,
    t_note: String,
    mem_kind_note: bool, // false = note, true = notify
    mem_match: String,
    mem_text: String,
    focus_handle: FocusHandle,
}

impl JeanView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            tab: JeanTab::Overview,
            state: None,
            history: Vec::new(),
            targets: None,
            memory: Vec::new(),
            detail: None,
            history_status: String::new(),
            loading: false,
            error: None,
            active_field: Field::None,
            say_input: String::new(),
            history_search: String::new(),
            t_domain: String::new(),
            t_host: String::new(),
            t_note: String::new(),
            mem_kind_note: true,
            mem_match: String::new(),
            mem_text: String::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn set_state(&mut self, state: JeanState) {
        self.state = Some(state);
        self.loading = false;
        self.error = None;
    }
    pub fn set_history(&mut self, history: Vec<JeanTicket>) {
        self.history = history;
        self.loading = false;
    }
    pub fn set_targets(&mut self, targets: JeanTargets) {
        self.targets = Some(targets);
        self.loading = false;
    }
    pub fn set_memory(&mut self, memory: Vec<JeanMemory>) {
        self.memory = memory;
        self.loading = false;
    }
    pub fn set_detail(&mut self, ticket: JeanTicket) {
        self.detail = Some(ticket);
        self.loading = false;
    }
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }
    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.error = Some(msg.into());
        self.loading = false;
    }

    fn field_buf(&mut self, f: Field) -> Option<&mut String> {
        match f {
            Field::Say => Some(&mut self.say_input),
            Field::HistorySearch => Some(&mut self.history_search),
            Field::TargetDomain => Some(&mut self.t_domain),
            Field::TargetHost => Some(&mut self.t_host),
            Field::TargetNote => Some(&mut self.t_note),
            Field::MemMatch => Some(&mut self.mem_match),
            Field::MemText => Some(&mut self.mem_text),
            Field::None => None,
        }
    }

    fn handle_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let active = self.active_field;
        if active == Field::None {
            return;
        }
        match key {
            "enter" => self.submit_active(cx),
            "backspace" => {
                if let Some(b) = self.field_buf(active) {
                    b.pop();
                    cx.notify();
                }
            }
            _ => {
                if let Some(ref kc) = event.keystroke.key_char {
                    if !event.keystroke.modifiers.control && !event.keystroke.modifiers.alt {
                        if let Some(b) = self.field_buf(active) {
                            b.push_str(kc);
                            cx.notify();
                        }
                    }
                } else if key.len() == 1
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                {
                    if let Some(b) = self.field_buf(active) {
                        b.push_str(key);
                        cx.notify();
                    }
                }
            }
        }
    }

    fn submit_active(&mut self, cx: &mut Context<Self>) {
        match self.active_field {
            Field::Say => self.submit_say(cx),
            Field::HistorySearch => {
                cx.emit(JeanViewEvent::LoadHistory {
                    q: self.history_search.trim().to_string(),
                    status: self.history_status.clone(),
                });
            }
            Field::TargetHost | Field::TargetDomain | Field::TargetNote => self.submit_target(cx),
            Field::MemMatch | Field::MemText => self.submit_memory(cx),
            Field::None => {}
        }
    }

    fn submit_say(&mut self, cx: &mut Context<Self>) {
        let text = self.say_input.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.say_input.clear();
        cx.emit(JeanViewEvent::Say(text));
        cx.notify();
    }

    fn submit_target(&mut self, cx: &mut Context<Self>) {
        let domain = self.t_domain.trim().to_string();
        let ssh_host = self.t_host.trim().to_string();
        if domain.is_empty() || ssh_host.is_empty() {
            return;
        }
        let note = self.t_note.trim().to_string();
        self.t_domain.clear();
        self.t_host.clear();
        self.t_note.clear();
        cx.emit(JeanViewEvent::AddTarget {
            domain,
            ssh_host,
            note,
        });
        cx.notify();
    }

    fn submit_memory(&mut self, cx: &mut Context<Self>) {
        let text = self.mem_text.trim().to_string();
        let match_ = self.mem_match.trim().to_string();
        if text.is_empty() && match_.is_empty() {
            return;
        }
        let kind = if self.mem_kind_note { "note" } else { "notify" };
        self.mem_match.clear();
        self.mem_text.clear();
        cx.emit(JeanViewEvent::AddMemory {
            kind: kind.to_string(),
            match_,
            text,
        });
        cx.notify();
    }

    // ── small building blocks ────────────────────────────────────────────

    fn input_box(
        &self,
        field: Field,
        value: &str,
        placeholder: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.active_field == field;
        let content = if value.is_empty() {
            div()
                .text_color(ShellDeckColors::text_muted())
                .child(placeholder.to_string())
        } else {
            div()
                .flex()
                .text_color(ShellDeckColors::text_primary())
                .child(value.to_string())
                .child(if active {
                    div().w(px(1.0)).h(px(15.0)).bg(ShellDeckColors::primary())
                } else {
                    div()
                })
        };
        div()
            .id(ElementId::from(SharedString::from(format!("jf-{placeholder}"))))
            .px(px(9.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .bg(ShellDeckColors::bg_primary())
            .border_1()
            .border_color(if active {
                ShellDeckColors::primary()
            } else {
                ShellDeckColors::border()
            })
            .text_size(px(13.0))
            .cursor_text()
            .child(content)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.active_field = field;
                cx.notify();
            }))
    }

    fn btn(
        id: &'static str,
        label: &str,
        cx: &mut Context<Self>,
        on: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        div()
            .id(id)
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
            .child(label.to_string())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| on(this, cx)))
    }

    fn tab_button(&self, tab: JeanTab, label: &str, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.tab == tab;
        let mut b = div()
            .id(ElementId::from(SharedString::from(format!("jtab-{label}"))))
            .px(px(12.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .text_size(px(13.0))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .child(label.to_string())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.tab = tab;
                match tab {
                    JeanTab::Targets => cx.emit(JeanViewEvent::LoadTargets),
                    JeanTab::Memory => cx.emit(JeanViewEvent::LoadMemory),
                    JeanTab::History => cx.emit(JeanViewEvent::LoadHistory {
                        q: this.history_search.trim().to_string(),
                        status: this.history_status.clone(),
                    }),
                    JeanTab::Overview => {}
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
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (connected, paused, max, chan) = self
            .state
            .as_ref()
            .map(|s| {
                (
                    s.bot.connected,
                    s.bot.paused,
                    s.bot.max,
                    if s.bot.channel_name.is_empty() {
                        s.bot.channel.clone()
                    } else {
                        format!("#{}", s.bot.channel_name)
                    },
                )
            })
            .unwrap_or((false, false, 0, String::new()));

        let dot = if connected {
            ShellDeckColors::success()
        } else {
            ShellDeckColors::error()
        };

        div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(10.0))
            .px(px(14.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(div().size(px(8.0)).rounded_full().bg(dot))
            .child(
                div()
                    .text_size(px(13.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(if connected {
                        "JeanClaude connecté".to_string()
                    } else {
                        "JeanClaude hors ligne".to_string()
                    }),
            )
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(chan),
            )
            .child(
                Self::btn(
                    "jean-pause",
                    if paused { "▶ Reprendre" } else { "⏸ Pause" },
                    cx,
                    move |_this, cx| cx.emit(JeanViewEvent::SetPaused(!paused)),
                )
                .mr(px(2.0)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child("Parallélisme"),
                    )
                    .child(Self::btn("jean-conc-dec", "−", cx, move |_t, cx| {
                        cx.emit(JeanViewEvent::SetConcurrency((max - 1).max(1)))
                    }))
                    .child(
                        div()
                            .min_w(px(20.0))
                            .flex()
                            .justify_center()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(max.to_string()),
                    )
                    .child(Self::btn("jean-conc-inc", "+", cx, move |_t, cx| {
                        cx.emit(JeanViewEvent::SetConcurrency(max + 1))
                    })),
            )
            .child(
                Self::btn("jean-refresh", "↻ Actualiser", cx, |_t, cx| {
                    cx.emit(JeanViewEvent::Refresh)
                }),
            )
    }

    fn render_say(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(14.0))
            .py(px(8.0))
            .child(div().flex_1().child(self.input_box(
                Field::Say,
                &self.say_input.clone(),
                "Dire dans #jean…",
                cx,
            )))
            .child(
                div()
                    .id("jean-say-send")
                    .px(px(12.0))
                    .py(px(7.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::primary())
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white())
                    .cursor_pointer()
                    .child("Envoyer")
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.submit_say(cx))),
            )
    }

    fn render_overview(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div()
            .id("jean-overview")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .p(px(14.0));

        // Pending confirmations.
        let pending = self.state.as_ref().map(|s| s.pending.clone()).unwrap_or_default();
        col = col.child(Self::section_title(&format!(
            "Confirmations en attente ({})",
            pending.len()
        )));
        if pending.is_empty() {
            col = col.child(Self::muted("Aucune confirmation en attente."));
        } else {
            for p in &pending {
                let thread = p.thread_ts.clone();
                let thread2 = p.thread_ts.clone();
                let who = p.author_name.clone().unwrap_or_default();
                let count_note = if p.count > 1 {
                    format!("  ({} tickets)", p.count)
                } else {
                    String::new()
                };
                col = col.child(
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
                                .text_size(px(13.0))
                                .text_color(ShellDeckColors::text_primary())
                                .child(format!("{}{}", p.prompt, count_note)),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(format!("de {}", who)),
                        )
                        .child(
                            div()
                                .flex()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .id(ElementId::from(SharedString::from(format!(
                                            "jc-ok-{thread}"
                                        ))))
                                        .px(px(10.0))
                                        .py(px(5.0))
                                        .rounded(px(6.0))
                                        .bg(ShellDeckColors::success())
                                        .text_size(px(12.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(white())
                                        .cursor_pointer()
                                        .child("✅ Confirmer")
                                        .on_click(cx.listener(move |_t, _: &ClickEvent, _, cx| {
                                            cx.emit(JeanViewEvent::Confirm(thread.clone()))
                                        })),
                                )
                                .child(
                                    div()
                                        .id(ElementId::from(SharedString::from(format!(
                                            "jc-no-{thread2}"
                                        ))))
                                        .px(px(10.0))
                                        .py(px(5.0))
                                        .rounded(px(6.0))
                                        .border_1()
                                        .border_color(ShellDeckColors::border())
                                        .text_size(px(12.0))
                                        .text_color(ShellDeckColors::error())
                                        .cursor_pointer()
                                        .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                                        .child("❌ Rejeter")
                                        .on_click(cx.listener(move |_t, _: &ClickEvent, _, cx| {
                                            cx.emit(JeanViewEvent::Reject(thread2.clone()))
                                        })),
                                ),
                        ),
                );
            }
        }

        // Active (running/queued) tickets.
        let tickets = self.state.as_ref().map(|s| s.tickets.clone()).unwrap_or_default();
        let active: Vec<&JeanTicket> = tickets
            .iter()
            .filter(|t| t.is_running() || t.is_queued())
            .collect();
        col = col.child(Self::section_title(&format!("Tickets actifs ({})", active.len())));
        if active.is_empty() {
            col = col.child(Self::muted("Aucun ticket en cours."));
        } else {
            let now = now_ms();
            for t in active {
                let id = t.id.clone();
                let stale = t
                    .heartbeat_age_ms(now)
                    .map(|a| a > 90_000.0)
                    .unwrap_or(false);
                let hb = t
                    .heartbeat_age_ms(now)
                    .map(|a| format!("battement il y a {}s", (a / 1000.0) as i64))
                    .unwrap_or_default();
                col = col.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .p(px(10.0))
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(ShellDeckColors::border())
                        .bg(ShellDeckColors::bg_sidebar())
                        .child(
                            div()
                                .flex_1()
                                .flex()
                                .flex_col()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(format!(
                                            "[{}] {}",
                                            t.status,
                                            t.activity.clone().unwrap_or_else(|| t.prompt.clone())
                                        )),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .gap(px(8.0))
                                        .child(
                                            div()
                                                .text_size(px(11.0))
                                                .text_color(ShellDeckColors::text_muted())
                                                .child(
                                                    t.target
                                                        .clone()
                                                        .map(|s| format!("🎯 {}", s))
                                                        .unwrap_or_default(),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.0))
                                                .text_color(if stale {
                                                    ShellDeckColors::warning()
                                                } else {
                                                    ShellDeckColors::text_muted()
                                                })
                                                .child(if stale {
                                                    format!("⚠ {}", hb)
                                                } else {
                                                    hb
                                                }),
                                        ),
                                ),
                        )
                        .child(Self::btn("jean-cancel", "Annuler", cx, move |_t, cx| {
                            cx.emit(JeanViewEvent::Cancel(id.clone()))
                        })),
                );
            }
        }

        col
    }

    fn render_history(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let statuses = ["", "running", "queued", "done", "error", "cancelled"];
        let mut filters = div().flex().flex_wrap().gap(px(4.0)).mb(px(8.0));
        for s in statuses {
            let active = self.history_status == s;
            let label = if s.is_empty() { "tous" } else { s };
            let sval = s.to_string();
            let mut chip = div()
                .id(ElementId::from(SharedString::from(format!("jhs-{label}"))))
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .text_size(px(11.0))
                .cursor_pointer()
                .hover(|x| x.bg(ShellDeckColors::hover_bg()))
                .child(label.to_string())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.history_status = sval.clone();
                    cx.emit(JeanViewEvent::LoadHistory {
                        q: this.history_search.trim().to_string(),
                        status: this.history_status.clone(),
                    });
                    cx.notify();
                }));
            if active {
                chip = chip
                    .bg(ShellDeckColors::selected_bg())
                    .text_color(ShellDeckColors::text_primary());
            } else {
                chip = chip.text_color(ShellDeckColors::text_muted());
            }
            filters = filters.child(chip);
        }

        let mut list = div().flex().flex_col();
        if self.history.is_empty() {
            list = list.child(Self::muted("Aucun ticket."));
        } else {
            for t in &self.history {
                let id = t.id.clone();
                let selected = self.detail.as_ref().map(|d| d.id == t.id).unwrap_or(false);
                let mut row = div()
                    .id(ElementId::from(SharedString::from(format!("jh-{}", t.id))))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                    .on_click(cx.listener(move |_t, _: &ClickEvent, _, cx| {
                        cx.emit(JeanViewEvent::SelectTicket(id.clone()))
                    }))
                    .child(status_pill(&t.status))
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(t.prompt.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t.author_name.clone().unwrap_or_default()),
                    );
                if selected {
                    row = row.bg(ShellDeckColors::selected_bg());
                }
                list = list.child(row);
            }
        }

        let left = div()
            .w(px(360.0))
            .flex_shrink_0()
            .h_full()
            .overflow_hidden()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .p(px(12.0))
            .child(self.input_box(
                Field::HistorySearch,
                &self.history_search.clone(),
                "Rechercher (Entrée)…",
                cx,
            ))
            .child(div().h(px(8.0)))
            .child(filters)
            .child(
                div()
                    .id("jean-hist-scroll")
                    .flex_1()
                    .overflow_y_scroll()
                    .child(list),
            );

        div()
            .flex_1()
            .flex()
            .min_h(px(0.0))
            .child(left)
            .child(self.render_detail(cx))
    }

    fn render_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(t) = self.detail.clone() else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_muted())
                .child("Sélectionnez un ticket");
        };
        let id_force = t.id.clone();
        let id_cancel = t.id.clone();
        let can_cancel = t.is_running() || t.is_queued();

        let mut actions = div()
            .id("jean-detail-actions")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(14.0));
        if t.actions.is_empty() {
            actions = actions.child(Self::muted("Aucune action enregistrée."));
        } else {
            for a in &t.actions {
                actions = actions.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_primary())
                        .child(format!("• {}", a)),
                );
            }
        }

        let mut meta = format!("[{}] {}", t.status, t.prompt);
        if let Some(tgt) = &t.target {
            meta.push_str(&format!("\n🎯 {}", tgt));
        }
        if let Some(url) = &t.ticket_url {
            meta.push_str(&format!("\n{}", url));
        }
        if let Some(err) = &t.error {
            meta.push_str(&format!("\n⚠ {}", err));
        }

        let mut bar = div()
            .flex()
            .gap(px(6.0))
            .px(px(14.0))
            .py(px(8.0))
            .border_t_1()
            .border_color(ShellDeckColors::border());
        bar = bar.child(Self::btn("jean-force", "Forcer (ticket)", cx, move |_t, cx| {
            cx.emit(JeanViewEvent::Force(id_force.clone()))
        }));
        if can_cancel {
            bar = bar.child(Self::btn("jean-detail-cancel", "Annuler", cx, move |_t, cx| {
                cx.emit(JeanViewEvent::Cancel(id_cancel.clone()))
            }));
        }

        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .child(
                div()
                    .px(px(14.0))
                    .py(px(10.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(meta),
            )
            .child(actions)
            .child(bar)
    }

    fn render_targets(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div().flex().flex_col().gap(px(2.0));
        if let Some(tg) = &self.targets {
            for (dom, rule) in tg.suffixes.iter().chain(tg.mappings.iter()) {
                let dom_c = dom.clone();
                let removable = tg.mappings.contains_key(dom);
                let mut row = div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(5.0))
                    .rounded(px(6.0))
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(format!("{} → {}", dom, rule.ssh_host)),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(rule.note.clone().unwrap_or_default()),
                    );
                if removable {
                    row = row.child(Self::btn("jean-tgt-rm", "×", cx, move |_t, cx| {
                        cx.emit(JeanViewEvent::RemoveTarget(dom_c.clone()))
                    }));
                }
                list = list.child(row);
            }
        }

        div()
            .id("jean-targets-scroll")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .p(px(14.0))
            .child(Self::section_title("Cibles apprises (domaine → serveur SSH)"))
            .child(list)
            .child(Self::section_title("Ajouter une cible"))
            .child(
                div()
                    .flex()
                    .gap(px(6.0))
                    .child(div().flex_1().child(self.input_box(
                        Field::TargetDomain,
                        &self.t_domain.clone(),
                        "domaine",
                        cx,
                    )))
                    .child(div().flex_1().child(self.input_box(
                        Field::TargetHost,
                        &self.t_host.clone(),
                        "serveur ssh",
                        cx,
                    )))
                    .child(div().flex_1().child(self.input_box(
                        Field::TargetNote,
                        &self.t_note.clone(),
                        "note (option)",
                        cx,
                    )))
                    .child(Self::btn("jean-tgt-add", "Ajouter", cx, |this, cx| {
                        this.submit_target(cx)
                    })),
            )
    }

    fn render_memory(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div().flex().flex_col().gap(px(4.0));
        if self.memory.is_empty() {
            list = list.child(Self::muted("Aucune mémoire."));
        } else {
            for m in &self.memory {
                let id = m.id.clone();
                let names = if m.notify_names.is_empty() {
                    String::new()
                } else {
                    format!("  → {}", m.notify_names.join(", "))
                };
                list = list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(8.0))
                        .py(px(5.0))
                        .rounded(px(6.0))
                        .child(
                            div()
                                .px(px(5.0))
                                .rounded(px(6.0))
                                .bg(ShellDeckColors::badge_bg())
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(m.kind.clone()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.0))
                                .text_color(ShellDeckColors::text_primary())
                                .child(format!(
                                    "{}{}{}",
                                    if m.match_.is_empty() {
                                        String::new()
                                    } else {
                                        format!("[{}] ", m.match_)
                                    },
                                    m.text,
                                    names
                                )),
                        )
                        .child(Self::btn("jean-mem-rm", "×", cx, move |_t, cx| {
                            cx.emit(JeanViewEvent::RemoveMemory(id.clone()))
                        })),
                );
            }
        }

        let kind_note = self.mem_kind_note;
        div()
            .id("jean-memory-scroll")
            .flex_1()
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .p(px(14.0))
            .child(Self::section_title("Mémoire (règles de notification + notes)"))
            .child(list)
            .child(Self::section_title("Ajouter"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div()
                            .id("jean-mem-kind")
                            .px(px(8.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .border_1()
                            .border_color(ShellDeckColors::border())
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_primary())
                            .cursor_pointer()
                            .child(if kind_note { "note" } else { "notify" })
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.mem_kind_note = !this.mem_kind_note;
                                cx.notify();
                            })),
                    )
                    .child(div().w(px(140.0)).child(self.input_box(
                        Field::MemMatch,
                        &self.mem_match.clone(),
                        "match (mot-clé)",
                        cx,
                    )))
                    .child(div().flex_1().child(self.input_box(
                        Field::MemText,
                        &self.mem_text.clone(),
                        "texte",
                        cx,
                    )))
                    .child(Self::btn("jean-mem-add", "Ajouter", cx, |this, cx| {
                        this.submit_memory(cx)
                    })),
            )
    }

    fn section_title(label: &str) -> impl IntoElement {
        div()
            .text_size(px(11.0))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(ShellDeckColors::text_muted())
            .child(label.to_string())
    }
    fn muted(label: &str) -> impl IntoElement {
        div()
            .py(px(6.0))
            .text_size(px(12.0))
            .text_color(ShellDeckColors::text_muted())
            .child(label.to_string())
    }
}

impl Render for JeanView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .px(px(12.0))
            .py(px(6.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(self.tab_button(JeanTab::Overview, "Aperçu", cx))
            .child(self.tab_button(JeanTab::History, "Historique", cx))
            .child(self.tab_button(JeanTab::Targets, "Cibles", cx))
            .child(self.tab_button(JeanTab::Memory, "Mémoire", cx));

        let body = match self.tab {
            JeanTab::Overview => self.render_overview(cx).into_any_element(),
            JeanTab::History => self.render_history(cx).into_any_element(),
            JeanTab::Targets => self.render_targets(cx).into_any_element(),
            JeanTab::Memory => self.render_memory(cx).into_any_element(),
        };

        let mut root = div()
            .id("jean-view-root")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _w, cx| this.handle_key(e, cx)))
            .size_full()
            .flex()
            .flex_col()
            .bg(ShellDeckColors::bg_primary())
            .child(self.render_status_bar(cx))
            .child(self.render_say(cx))
            .child(tabs)
            .child(body);

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

fn now_ms() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

fn status_pill(status: &str) -> impl IntoElement {
    let color = match status {
        "running" => ShellDeckColors::warning(),
        "done" => ShellDeckColors::success(),
        "error" => ShellDeckColors::error(),
        _ => ShellDeckColors::text_muted(),
    };
    div()
        .flex_shrink_0()
        .px(px(5.0))
        .py(px(1.0))
        .rounded(px(6.0))
        .bg(color.opacity(0.15))
        .text_size(px(10.0))
        .text_color(color)
        .child(status.to_string())
}
