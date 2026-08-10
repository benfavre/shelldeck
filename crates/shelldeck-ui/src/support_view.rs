//! Support mode — a native staff home plus a two-pane helpdesk console over the
//! token-gated support API. Tickets keep filters/list on the left and the
//! conversation, composer, and triage actions on the right.
//!
//! The view holds data and captures composer text; all network happens in the
//! `Workspace` (background executor) driven by [`SupportViewEvent`].

mod home;
mod issue_filters;
mod requests;
mod thread;
mod ticket_filters;
mod tickets;

use crate::attachment_annotator::AttachmentAnnotator;
use crate::i18n::rel_time;
use crate::icons::{lucide_icon, lucide_path};
use crate::issue_attachments::{
    AttachmentDraft, AttachmentLightbox, capture_region, draft_from_clipboard_image,
    render_attachment_draft_gallery, render_stored_attachment_gallery,
};
use crate::scale::px;
use adabraka_ui::components::avatar::{Avatar, AvatarSize};
use adabraka_ui::components::button::{Button, ButtonSize, ButtonVariant};
use adabraka_ui::components::checkbox::Checkbox;
use adabraka_ui::components::confirm_dialog::Dialog as UiDialog;
use adabraka_ui::components::icon_button::IconButton;
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize, InputState, Paste};
use adabraka_ui::components::label::Label;
use adabraka_ui::components::select::{Select, SelectOption};
use adabraka_ui::display::badge::{Badge, BadgeVariant};
use adabraka_ui::display::card::Card;
use adabraka_ui::overlays::popover::{Popover, PopoverContent};
use adabraka_ui::overlays::popover_menu::{PopoverMenu, PopoverMenuItem};
use adabraka_ui::prelude::scrollable_vertical;
use gpui::prelude::*;
use gpui::*;
use std::ops::Range;
use std::rc::Rc;

use shelldeck_core::config::issues::{
    ISSUE_ATTACHMENT_MAX_COUNT, Issue, IssueAttachment, IssueInstance,
};
use shelldeck_core::config::manage_support::{
    SupportAgent, SupportCounts, SupportMe, SupportMessage, SupportTicket,
};

use crate::t;
use crate::theme::ShellDeckColors;

/// Which section of the support console is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportSection {
    Home,
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

/// Shared 26 px footer action used by both Support composers. adabraka's
/// smallest labeled `Button` is 36 px, which makes the footer taller than the
/// writing field; the shared Composer deliberately exposes custom action slots
/// for this denser chrome.
fn compact_composer_action(
    id: &'static str,
    icon: &'static str,
    label: impl Into<SharedString>,
    enabled: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(26.0))
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(5.0))
        .px(px(6.0))
        .rounded(px(7.0))
        .text_size(px(11.5))
        .text_color(ShellDeckColors::text_muted())
        .when(enabled, |action| {
            action.cursor_pointer().hover(|style| {
                style
                    .bg(ShellDeckColors::hover_bg())
                    .text_color(ShellDeckColors::text_primary())
            })
        })
        .when(!enabled, |action| action.opacity(0.5))
        .child(
            svg()
                .path(lucide_path(icon))
                .size(px(14.0))
                .flex_shrink_0()
                .text_color(ShellDeckColors::text_muted()),
        )
        .child(label.into())
}

/// Message destination in the ticket Composer option slot. Requests use that
/// slot for their AI model because their API has no destination choice.
fn composer_delivery_chip(
    id: &'static str,
    icon: &'static str,
    label: impl Into<SharedString>,
    interactive: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(26.0))
        .flex()
        .flex_shrink_0()
        .items_center()
        .gap(px(5.0))
        .px(px(7.0))
        .rounded(px(7.0))
        .text_size(px(11.0))
        .text_color(ShellDeckColors::text_muted())
        .when(interactive, |chip| {
            chip.cursor_pointer()
                .hover(|style| style.bg(ShellDeckColors::hover_bg()))
        })
        .child(
            svg()
                .path(lucide_path(icon))
                .size(px(12.0))
                .flex_shrink_0()
                .text_color(ShellDeckColors::text_muted()),
        )
        .child(label.into())
        .when(interactive, |chip| {
            chip.child(
                svg()
                    .path(lucide_path("chevron-down"))
                    .size(px(11.0))
                    .flex_shrink_0()
                    .text_color(ShellDeckColors::text_muted()),
            )
        })
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
    AdvPriorityOpt { value: Some("low") },
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

#[derive(Clone, Debug)]
enum AttachmentDeleteTarget {
    Ticket {
        target_id: String,
        attachment_id: String,
    },
    Issue {
        target_id: String,
        attachment_id: String,
    },
}

/// Flattened rows consumed by GPUI's variable-height list. Timeline objects
/// stay semantic here so day boundaries and transient future API state do not
/// have to masquerade as comments.
#[derive(Debug, Clone)]
enum IssueThreadRow {
    Opening {
        block: usize,
        last: bool,
    },
    Comment {
        comment: usize,
        block: usize,
        first: bool,
        last: bool,
    },
    Day {
        at: f64,
    },
    Typing {
        index: usize,
    },
    AiDraft,
    LocalDraft,
}

/// Requests the view raises for the workspace to service (all network).
#[derive(Debug, Clone)]
pub enum SupportViewEvent {
    Refresh,
    SelectTicket(String),
    SuggestReply(String),
    SummarizeTicket(String),
    TriageTicket(String),
    SuggestIssueReply(String),
    /// The AI draft's Publier button — prepends the draft into the composer.
    PublishIssueAiDraft,
    /// The AI draft's Rejeter button.
    DiscardIssueAiDraft,
    SummarizeIssue(String),
    TriageIssue(String),
    /// Send the composer text as a reply (note=false) or internal note (note=true).
    Send {
        id: String,
        text: String,
        note: bool,
        attachments: Vec<AttachmentDraft>,
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
        attachments: Vec<AttachmentDraft>,
    },
    /// Reserved for the future resend endpoint. The current API exposes the
    /// failed delivery state but has no idempotent retry operation yet.
    RetryIssueComment {
        issue_id: String,
        comment_id: String,
    },
    ImportAttachmentUrl {
        url: String,
        generation: u64,
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
    /// The reply composer's provider chip. Support does not own `ai.*`, so the
    /// Workspace persists it through Settings like every other surface.
    SelectAiBackend(shelldeck_core::ai::AiBackend),
    IssueGithubPush(String),
    IssueGithubRefresh(String),
    /// Soft-delete a request (staff only — confirmed via a dialog first).
    IssueDelete(String),
    IssueAttachmentDelete {
        id: String,
        attachment_id: String,
    },
    SupportAttachmentDelete {
        id: String,
        attachment_id: String,
    },
    /// Any filter changed — simple bar (status chip / search) or advanced
    /// modal apply. Carries the full filter payload so the Workspace stores
    /// one canonical value; empty strings / `None` fields mean "no filter
    /// on that leg" and get omitted from the request.
    IssuesFilterChanged {
        filter: shelldeck_core::config::issues::IssueListFilter,
    },
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
    /// Full editor state backing ticket replies and request comments.
    composer_state: Entity<InputState>,
    /// Pending AI reply (issue only, for now). Kept OUT of `composer_state` so
    /// it does not shove aside what the user was writing — the mockup shows it
    /// as a distinct card above the composer, with Publier / Modifier /
    /// Régénérer / Rejeter.
    issue_ai_draft: Option<AiDraft>,
    /// Whether the last `SuggestIssueReply` is still running, so the ✦ button
    /// stays honest — no spinner text, just disable.
    issue_ai_pending: bool,
    attachment_url_state: Entity<InputState>,
    /// Reveals the optional URL importer only when explicitly requested.
    attachment_url_open: bool,
    attachment_drafts: Vec<AttachmentDraft>,
    attachment_panel_open: bool,
    attachment_busy: bool,
    attachment_generation: u64,
    compose_note: bool,
    ai_reply_enabled: bool,
    ai_issue_enabled: bool,
    /// Which backend answers here. Support only knew *whether* AI was on, not
    /// *which* one — so the reply composer had no way to show or change it.
    ai_backend: shelldeck_core::ai::AiBackend,
    ai_model: String,
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
    /// Signed-in account identity, **pre-lowercased and trimmed** — used to
    /// gate the delete action on requests the current user filed themselves
    /// and to render self-authored comments right-aligned. Kept in sync with
    /// `AppConfig.account` via `set_account` (workspace pushes on login,
    /// logout, and every issues refresh). Lowercased once here so
    /// `is_my_issue` / `render_issue_comment` avoid re-normalising per row —
    /// the pre-normalised form serves both correctness (single owner) and
    /// perf (list of 50 issues × 20 comments used to alloc ~2000 strings
    /// per paint).
    account_name_lc: String,
    account_email_lc: String,
    /// Applied filter — mirrors `Workspace::issues_filter` so the chips /
    /// input reflect the state the server was actually queried with. The
    /// modal draft below is only populated while the "Filtres" modal is
    /// open (opened via `open_issues_filter_modal`, applied on OK, dropped
    /// on Reset / close).
    issues_filter: shelldeck_core::config::issues::IssueListFilter,
    issues_filter_draft: shelldeck_core::config::issues::IssueListFilter,
    issues_filter_modal_open: bool,
    /// Nested picker modal for the issues advanced filter — opened when
    /// the user clicks the assignee button inside the filter modal. Uses
    /// a full modal (with search) rather than a Select dropdown because
    /// agent lists can grow, and a searchable overlay is cleaner than a
    /// cramped popover.
    issues_assignee_modal_open: bool,
    issues_assignee_search_state: Entity<InputState>,
    /// Search state for the compact header assignee popover. Kept separate
    /// from the advanced-filter modal so opening either picker cannot leak a
    /// query into the other.
    issue_assignee_search_state: Entity<InputState>,
    issues_search_state: Entity<InputState>,
    issue_instances: Vec<IssueInstance>,
    issue_detail: Option<Issue>,
    issue_selected: Option<String>,
    /// Kebab menu anchor for a request. Carries the issue id + click position
    /// so both the list-row kebab (works without opening the detail) and the
    /// detail-header kebab share the same popover machinery.
    issue_popover_menu: Option<(String, Point<Pixels>)>,
    /// Request id pending a confirmed soft-delete (drives the confirm modal).
    confirm_issue_delete: Option<String>,
    confirm_attachment_delete: Option<AttachmentDeleteTarget>,
    /// Native full-screen preview for images attached to the open request.
    attachment_lightbox: Option<Entity<AttachmentLightbox>>,
    /// Annotation editor opened after an interactive area capture.
    capture_annotator: Option<Entity<AttachmentAnnotator>>,
    /// Parsed top-level Markdown blocks for the opening request and each
    /// comment. The variable-height GPUI list renders only visible blocks.
    issue_body_blocks: Vec<SharedString>,
    issue_comment_blocks: Vec<Vec<SharedString>>,
    issue_thread_rows: Vec<IssueThreadRow>,
    issue_thread_list: ListState,
    focus_handle: FocusHandle,
    /// Parsed Markdown blocks and variable-height list state for tickets.
    /// Bottom alignment opens on the latest exchange without rendering the
    /// complete history.
    ticket_message_blocks: Vec<Vec<SharedString>>,
    ticket_thread_list: ListState,
}

impl SupportView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let parent = cx.entity();
        let assignee_draft_select =
            Self::build_assignee_draft_select(None, &[], parent.clone(), cx);
        let composer_state = cx.new(|cx| InputState::new(cx).multi_line(true));
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
            issue_ai_draft: None,
            issue_ai_pending: false,
            composer_state,
            attachment_url_state: cx.new(InputState::new),
            attachment_url_open: false,
            attachment_drafts: Vec::new(),
            attachment_panel_open: false,
            attachment_busy: false,
            attachment_generation: 0,
            compose_note: false,
            ai_reply_enabled: false,
            ai_issue_enabled: false,
            ai_backend: shelldeck_core::ai::AiBackend::Disabled,
            ai_model: String::new(),
            loading: false,
            error: None,
            assign_menu_open: false,
            priority_menu_open: false,
            popover_menu: None,
            jean_available: false,
            jean_pending: Vec::new(),
            jean_active: 0,
            section: SupportSection::Home,
            issues: Vec::new(),
            issues_staff: false,
            account_name_lc: String::new(),
            account_email_lc: String::new(),
            issues_filter: shelldeck_core::config::issues::IssueListFilter::default(),
            issues_filter_draft: shelldeck_core::config::issues::IssueListFilter::default(),
            issues_filter_modal_open: false,
            issues_assignee_modal_open: false,
            issues_assignee_search_state: cx.new(InputState::new),
            issue_assignee_search_state: cx.new(InputState::new),
            issues_search_state: cx.new(InputState::new),
            issue_instances: Vec::new(),
            issue_detail: None,
            issue_selected: None,
            issue_popover_menu: None,
            confirm_issue_delete: None,
            confirm_attachment_delete: None,
            attachment_lightbox: None,
            capture_annotator: None,
            issue_body_blocks: Vec::new(),
            issue_comment_blocks: Vec::new(),
            issue_thread_rows: Vec::new(),
            issue_thread_list: ListState::new(0, ListAlignment::Bottom, gpui::px(320.0)),
            focus_handle: cx.focus_handle(),
            ticket_message_blocks: Vec::new(),
            ticket_thread_list: ListState::new(0, ListAlignment::Bottom, gpui::px(320.0)),
        }
    }

    /// Switch the console section (palette / action shortcut to Demandes).
    pub fn set_section(&mut self, section: SupportSection) {
        if self.section != section {
            self.attachment_generation = self.attachment_generation.wrapping_add(1);
            self.attachment_busy = false;
            self.attachment_drafts.clear();
            self.attachment_panel_open = false;
        }
        self.section = section;
    }

    pub fn set_issues(&mut self, issues: Vec<Issue>, staff: bool, instances: Vec<IssueInstance>) {
        self.issues = issues;
        self.issues_staff = staff;
        self.issue_instances = instances;
    }

    /// Update the signed-in account identity used by `is_my_issue` and
    /// `render_issue_comment`. Called from `Workspace::push_issues_to_support`
    /// alongside `set_issues`, and also on login / logout so the cache never
    /// outlives the workspace-owned `AppConfig.account`. Empty strings on
    /// logout — `is_my_issue` returns `false` when either half is empty.
    ///
    /// Inputs are normalised once (trim + lowercase) so per-row comparisons
    /// stay allocation-free.
    pub fn set_account(&mut self, name: &str, email: &str) {
        self.account_name_lc = name.trim().to_ascii_lowercase();
        self.account_email_lc = email.trim().to_ascii_lowercase();
    }

    /// Reset every "which row is open" bit so the Support surface returns to
    /// its list view. Called by the Workspace on mode switch so a ticket or a
    /// request opened in Support doesn't visually leak into User mode.
    pub fn clear_selection(&mut self) {
        self.attachment_generation = self.attachment_generation.wrapping_add(1);
        self.attachment_busy = false;
        self.attachment_drafts.clear();
        self.attachment_panel_open = false;
        self.attachment_url_open = false;
        self.capture_annotator = None;
        self.selected_id = None;
        self.detail = None;
        self.issue_selected = None;
        self.issue_detail = None;
        self.issue_body_blocks.clear();
        self.issue_comment_blocks.clear();
        self.issue_thread_rows.clear();
        self.issue_thread_list.reset(0);
        self.ticket_message_blocks.clear();
        self.ticket_thread_list.reset(0);
        self.popover_menu = None;
        self.priority_menu_open = false;
        self.assign_menu_open = false;
        self.issue_popover_menu = None;
        self.confirm_issue_delete = None;
    }

    pub fn set_issue_detail(&mut self, detail: Option<Issue>, cx: &mut Context<Self>) {
        let next_id = detail.as_ref().map(|issue| issue.id.as_str());
        let same_issue = next_id == self.issue_selected.as_deref();
        let detail_changed = self.issue_detail.as_ref() != detail.as_ref();
        let seeded_ai_draft = detail
            .as_ref()
            .and_then(|issue| issue.thread_state.suggested_reply.as_ref())
            .filter(|draft| !draft.body.trim().is_empty())
            .map(|draft| AiDraft {
                body: draft.body.clone(),
                model: draft.model.clone(),
            });
        if next_id != self.issue_selected.as_deref() {
            self.attachment_generation = self.attachment_generation.wrapping_add(1);
            self.attachment_busy = false;
            self.attachment_drafts.clear();
            self.attachment_panel_open = false;
            self.attachment_url_open = false;
            self.capture_annotator = None;
            self.issue_ai_draft = seeded_ai_draft;
            self.reset_composer(cx);
        }
        if let Some(d) = &detail {
            self.issue_selected = Some(d.id.clone());
        }
        self.issue_detail = detail;
        if detail_changed {
            self.rebuild_issue_thread_cache(same_issue);
        }
        self.issue_popover_menu = None;
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

    /// Count of tickets with `unread=true`. Used by the system tray
    /// counter row + the OS notification hook so external surfaces
    /// don't need to touch the private `tickets` field.
    pub fn unread_ticket_count(&self) -> usize {
        self.tickets.iter().filter(|t| t.unread).count()
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
        self.rebuild_ticket_thread_cache();
        self.popover_menu = None;
        self.priority_menu_open = false;
        self.assign_menu_open = false;
        self.reset_composer(cx);
        self.clear_attachment_drafts(cx);
        self.loading = false;
        self.error = None;
    }

    fn reset_composer(&self, cx: &mut Context<Self>) {
        self.composer_state.update(cx, |s, cx| {
            s.reset(cx);
        });
    }

    fn clear_attachment_drafts(&mut self, cx: &mut Context<Self>) {
        self.attachment_generation = self.attachment_generation.wrapping_add(1);
        self.attachment_busy = false;
        self.attachment_drafts.clear();
        self.attachment_panel_open = false;
        self.attachment_url_open = false;
        self.capture_annotator = None;
        self.attachment_url_state
            .update(cx, |state, cx| state.reset(cx));
    }

    fn add_attachment_draft(&mut self, draft: AttachmentDraft, cx: &mut Context<Self>) {
        if self.attachment_drafts.len() >= ISSUE_ATTACHMENT_MAX_COUNT {
            self.error = Some(
                t!(
                    "toast.issue.attachment_limit",
                    count = ISSUE_ATTACHMENT_MAX_COUNT
                )
                .to_string(),
            );
        } else {
            self.attachment_drafts.push(draft);
            self.attachment_panel_open = true;
            self.error = None;
        }
        cx.notify();
    }

    fn import_attachment_paths(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if generation != self.attachment_generation || self.attachment_busy {
            return;
        }
        let remaining = ISSUE_ATTACHMENT_MAX_COUNT.saturating_sub(self.attachment_drafts.len());
        if remaining == 0 {
            self.error = Some(
                t!(
                    "toast.issue.attachment_limit",
                    count = ISSUE_ATTACHMENT_MAX_COUNT
                )
                .to_string(),
            );
            cx.notify();
            return;
        }
        self.attachment_busy = true;
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let loaded = cx
                .background_executor()
                .spawn(async move {
                    paths
                        .into_iter()
                        .take(remaining)
                        .map(|path| AttachmentDraft::from_path(&path))
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.attachment_busy = false;
                if generation != view.attachment_generation {
                    return;
                }
                for result in loaded {
                    match result {
                        Ok(draft) => view.add_attachment_draft(draft, cx),
                        Err(error) => view.error = Some(error),
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn pick_attachments(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.attachment_busy {
            return;
        }
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t!("user.requests.attachments.choose").to_string().into()),
            starting_directory: None,
        });
        let generation = self.attachment_generation;
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let _ = this.update(cx, |view, cx| {
                view.import_attachment_paths(paths, generation, cx)
            });
        })
        .detach();
    }

    fn paste_attachment(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let Some(image) = item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image),
            _ => None,
        }) else {
            return false;
        };
        match draft_from_clipboard_image(image) {
            Ok(draft) => self.add_attachment_draft(draft, cx),
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
        true
    }

    fn capture_attachment(&mut self, cx: &mut Context<Self>) {
        if self.attachment_busy {
            return;
        }
        self.attachment_busy = true;
        let generation = self.attachment_generation;
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { capture_region() })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.attachment_busy = false;
                if generation != view.attachment_generation {
                    return;
                }
                match result {
                    Ok(draft) => view.open_capture_annotator(draft, cx),
                    Err(error) => view.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn open_capture_annotator(&mut self, draft: AttachmentDraft, cx: &mut Context<Self>) {
        let parent = cx.entity().downgrade();
        let cancel_parent = parent.clone();
        let apply_parent = parent;
        let annotator = cx.new(|cx| {
            AttachmentAnnotator::new(
                draft,
                move |cx| {
                    if let Some(parent) = cancel_parent.upgrade() {
                        parent.update(cx, |this, cx| {
                            this.capture_annotator = None;
                            cx.notify();
                        });
                    }
                },
                move |draft, cx| {
                    if let Some(parent) = apply_parent.upgrade() {
                        parent.update(cx, |this, cx| {
                            this.capture_annotator = None;
                            this.add_attachment_draft(draft, cx);
                        });
                    }
                },
                cx,
            )
        });
        self.capture_annotator = Some(annotator);
        cx.notify();
    }

    fn import_attachment_url(&mut self, cx: &mut Context<Self>) {
        let url = self
            .attachment_url_state
            .read(cx)
            .content()
            .trim()
            .to_string();
        if url.is_empty() || self.attachment_busy {
            return;
        }
        self.attachment_busy = true;
        cx.emit(SupportViewEvent::ImportAttachmentUrl {
            url,
            generation: self.attachment_generation,
        });
        cx.notify();
    }

    pub fn finish_attachment_url_import(
        &mut self,
        generation: u64,
        result: Result<AttachmentDraft, String>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.attachment_generation {
            return;
        }
        self.attachment_busy = false;
        match result {
            Ok(draft) => {
                self.attachment_url_state
                    .update(cx, |state, cx| state.reset(cx));
                self.attachment_url_open = false;
                self.add_attachment_draft(draft, cx);
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    pub fn clear_composer_after_send(&mut self, cx: &mut Context<Self>) {
        self.reset_composer(cx);
        self.clear_attachment_drafts(cx);
        self.loading = false;
        cx.notify();
    }

    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    pub fn set_error(&mut self, msg: impl Into<String>) {
        self.error = Some(msg.into());
        self.loading = false;
        self.attachment_busy = false;
    }

    pub fn selected_id(&self) -> Option<String> {
        self.selected_id.clone()
    }

    pub fn set_ai_backend(
        &mut self,
        backend: shelldeck_core::ai::AiBackend,
        model: String,
        cx: &mut Context<Self>,
    ) {
        if self.ai_backend != backend || self.ai_model != model {
            self.ai_backend = backend;
            self.ai_model = model;
            cx.notify();
        }
    }

    pub fn set_ai_reply_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.ai_reply_enabled = enabled;
        cx.notify();
    }

    pub fn set_ai_issue_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.ai_issue_enabled = enabled;
        cx.notify();
    }

    /// Called by `Workspace::route_ai_workflow_result` when an issue reply
    /// suggestion comes back — the AI writes into a distinct card so it never
    /// clobbers the user's in-flight text.
    pub fn set_issue_ai_draft(&mut self, text: String, cx: &mut Context<Self>) {
        self.issue_ai_pending = false;
        let text = text.trim().to_string();
        if text.is_empty() {
            self.issue_ai_draft = None;
        } else {
            self.issue_ai_draft = Some(AiDraft {
                body: text,
                model: self.ai_model.clone(),
            });
        }
        self.rebuild_issue_thread_cache(true);
        cx.notify();
    }

    pub fn set_issue_ai_pending(&mut self, pending: bool, cx: &mut Context<Self>) {
        self.issue_ai_pending = pending;
        if pending {
            self.issue_ai_draft = None;
            self.rebuild_issue_thread_cache(true);
        }
        cx.notify();
    }

    /// The user accepted the AI draft: promote it to the composer (respecting
    /// what is already there — we prefix, we do not clobber).
    pub fn publish_issue_ai_draft(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = self.issue_ai_draft.take() else {
            return;
        };
        self.rebuild_issue_thread_cache(true);
        let current = self.composer_state.read(cx).content().trim().to_string();
        let merged = if current.is_empty() {
            draft.body
        } else {
            format!("{}\n\n{}", current, draft.body)
        };
        self.composer_state.update(cx, |state, cx| {
            state.replace_content(merged, cx);
        });
        cx.notify();
    }

    pub fn discard_issue_ai_draft(&mut self, cx: &mut Context<Self>) {
        self.issue_ai_draft = None;
        self.rebuild_issue_thread_cache(true);
        cx.notify();
    }

    pub fn set_composer_draft(&mut self, text: String, cx: &mut Context<Self>) {
        self.compose_note = false;
        self.composer_state
            .update(cx, |state, cx| state.replace_content(text, cx));
        cx.notify();
    }

    pub fn ai_context_data(&self) -> serde_json::Value {
        match self.section {
            SupportSection::Home => serde_json::json!({
                "tickets": self.counts,
                "requests": self.issues.len(),
            }),
            SupportSection::Tickets => serde_json::to_value(&self.detail)
                .unwrap_or_else(|_| serde_json::json!({ "ticket": null })),
            SupportSection::Requests => serde_json::to_value(&self.issue_detail)
                .unwrap_or_else(|_| serde_json::json!({ "issue": null })),
        }
    }

    pub fn issue_triage_context_data(&self) -> serde_json::Value {
        serde_json::json!({
            "issue": self.issue_detail,
            "agents": self.agents.iter().take(50).map(|agent| serde_json::json!({
                "name": agent.name,
                "email": agent.email,
            })).collect::<Vec<_>>(),
            "current_user": {
                "name": self.me.name,
                "email": self.me.email,
            },
        })
    }

    pub fn support_triage_context_data(&self) -> serde_json::Value {
        serde_json::json!({
            "ticket": self.detail,
            "agents": self.agents.iter().take(50).map(|agent| serde_json::json!({
                "name": agent.name,
                "email": agent.email,
            })).collect::<Vec<_>>(),
            "current_user": {
                "name": self.me.name,
                "email": self.me.email,
            },
        })
    }

    pub fn selected_ticket_triage_state(&self) -> Option<(String, String)> {
        let ticket = self.detail.as_ref()?;
        Some((ticket.priority.clone(), ticket.assignee.clone()))
    }

    pub fn is_known_issue_assignee(&self, assignee: &str) -> bool {
        let assignee = assignee.trim();
        assignee.is_empty()
            || self
                .agents
                .iter()
                .any(|agent| agent.email.eq_ignore_ascii_case(assignee))
    }

    pub fn is_known_support_assignee(&self, assignee: &str) -> bool {
        self.is_known_issue_assignee(assignee)
    }

    pub fn ai_surface(&self) -> shelldeck_core::ai::AiSurface {
        match self.section {
            SupportSection::Home => shelldeck_core::ai::AiSurface::Support,
            SupportSection::Tickets => shelldeck_core::ai::AiSurface::Support,
            SupportSection::Requests => shelldeck_core::ai::AiSurface::Issue,
        }
    }

    /// Read the composer content once and, if non-empty, emit the right event
    /// (reply / note / issue comment). Multiline composers send explicitly
    /// through their button so Enter remains available for new lines.
    pub fn send_composer(&mut self, cx: &mut Context<Self>) {
        let text = self.composer_state.read(cx).content().trim().to_string();
        if text.is_empty() && self.attachment_drafts.is_empty() {
            return;
        }
        if self.attachment_busy {
            return;
        }
        let attachments = self.attachment_drafts.clone();
        match self.section {
            SupportSection::Home => {}
            SupportSection::Tickets => {
                if let Some(id) = self.selected_id.clone() {
                    self.attachment_busy = true;
                    let note = self.compose_note;
                    self.loading = true;
                    cx.emit(SupportViewEvent::Send {
                        id,
                        text,
                        note,
                        attachments,
                    });
                    cx.notify();
                }
            }
            SupportSection::Requests => {
                if let Some(id) = self.issue_selected.clone() {
                    self.attachment_busy = true;
                    cx.emit(SupportViewEvent::IssueComment {
                        id,
                        body: text,
                        attachments,
                    });
                    cx.notify();
                }
            }
        }
    }

    pub fn selected_ticket_identity(&self) -> Option<(String, String)> {
        let id = self.selected_id.as_ref()?;
        let label = self
            .detail
            .as_ref()
            .filter(|ticket| &ticket.id == id)
            .map(|ticket| ticket.subject.trim())
            .filter(|subject| !subject.is_empty())
            .unwrap_or(id)
            .to_string();
        Some((id.clone(), label))
    }

    // ── render helpers ───────────────────────────────────────────────────

    /// Compact JeanClaude strip: pending confirmations (confirm/reject inline)
    /// + active-ticket count. Shown only when Jean config is present.
    fn render_section_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab =
            |label: String, icon: &'static str, section: SupportSection, cx: &mut Context<Self>| {
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
                        if this.section != section {
                            this.clear_attachment_drafts(cx);
                        }
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
                t!("support.home.tab").to_string(),
                "house",
                SupportSection::Home,
                cx,
            ))
            .child(tab(
                t!("support.tickets").to_string(),
                "inbox",
                SupportSection::Tickets,
                cx,
            ))
            .child(tab(
                t!("support.requests_count", count = self.visible_issue_count()).to_string(),
                "tag",
                SupportSection::Requests,
                cx,
            ))
    }
}

impl Render for SupportView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search_query(cx);
        let filtered_count = self
            .tickets
            .iter()
            .filter(|t| self.passes_filter(t) && self.passes_advanced(t, &query))
            .count();

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

        let list = if filtered_count == 0 {
            div()
                .id("support-ticket-list-empty")
                .flex_1()
                .child(
                    div()
                        .p(px(16.0))
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(if self.has_list_constraints(cx) {
                            t!("support.empty.tickets_filtered").to_string()
                        } else {
                            t!("support.empty.tickets_view").to_string()
                        }),
                )
                .into_any_element()
        } else {
            uniform_list(
                "support-ticket-list",
                filtered_count,
                cx.processor(|this, range: Range<usize>, _window, cx| {
                    let query = this.search_query(cx);
                    let filtered_indices = this
                        .tickets
                        .iter()
                        .enumerate()
                        .filter(|(_, ticket)| {
                            this.passes_filter(ticket) && this.passes_advanced(ticket, &query)
                        })
                        .map(|(index, _)| index)
                        .collect::<Vec<_>>();
                    range
                        .filter_map(|index| filtered_indices.get(index).copied())
                        .filter_map(|index| this.tickets.get(index))
                        .map(|ticket| this.render_ticket_row(ticket, cx).into_any_element())
                        .collect::<Vec<_>>()
                }),
            )
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .into_any_element()
        };

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
            SupportSection::Home => self.render_home(cx).into_any_element(),
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

        if self.section == SupportSection::Requests && self.issues_filter_modal_open {
            root = root.child(self.render_issues_filter_modal(cx));
        }
        if self.section == SupportSection::Requests && self.issues_assignee_modal_open {
            root = root.child(self.render_issues_assignee_picker_modal(cx));
        }

        if self.section == SupportSection::Requests {
            if let Some((iid, pos)) = self.issue_popover_menu.clone() {
                // Prefer the open detail (may have fresher fields than the
                // list slim) — fall back to the list row (for row kebabs
                // fired without opening the detail).
                let iss = self
                    .issue_detail
                    .as_ref()
                    .filter(|d| d.id == iid)
                    .cloned()
                    .or_else(|| self.issues.iter().find(|i| i.id == iid).cloned());
                if let Some(iss) = iss {
                    root = root.child(self.render_issue_popover(&iss, pos, cx));
                }
            }
            if let Some(id) = self.confirm_issue_delete.clone() {
                root = root.child(self.render_delete_issue_modal(id, cx));
            }
        }
        if let Some(target) = self.confirm_attachment_delete.clone() {
            root = root.child(self.render_delete_attachment_modal(target, cx));
        }

        if let Some(lightbox) = &self.attachment_lightbox {
            root = root.child(lightbox.clone());
        }

        if let Some(annotator) = &self.capture_annotator {
            root = root.child(annotator.clone());
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

#[derive(Debug, Clone)]
pub struct AiDraft {
    pub body: String,
    pub model: String,
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
#[expect(dead_code)]
fn next_priority(p: &str) -> &'static str {
    match p {
        "low" => "normal",
        "normal" => "high",
        "high" => "urgent",
        _ => "low",
    }
}

/// Shared "delete request" destructive confirm — used from both
/// `SupportView` and `Workspace` (User mode also shows this modal on
/// requests filed by the current user). Callers pass a resolved
/// `title` (already looked up in whichever list they own) plus the
/// close / confirm actions; the visuals (trash-2 icon, red destructive
/// button, irreversible warning) are shared.
///
/// `id_prefix` scopes the button IDs so both surfaces can be alive at
/// once without adabraka's ElementId collision (see support/workspace
/// prefixes at the two call sites).
pub(crate) fn render_issue_delete_dialog(
    title: SharedString,
    id_prefix: &'static str,
    on_close: impl Fn(&mut App) + Clone + 'static,
    on_confirm: impl Fn(&mut App) + Clone + 'static,
) -> impl IntoElement {
    let body_line = if title.trim().is_empty() {
        t!("support.delete.body_generic").to_string()
    } else {
        t!("support.delete.body", title = title.to_string()).to_string()
    };

    let backdrop_close = on_close.clone();
    let cancel_close = on_close;
    let cancel_id: SharedString = format!("{id_prefix}-cancel").into();
    let confirm_id: SharedString = format!("{id_prefix}-confirm").into();

    UiDialog::new()
        .width(gpui::px(400.0))
        .on_backdrop_click(move |_, cx| backdrop_close(cx))
        .header(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(16.0))
                .py(px(14.0))
                .child(lucide_icon("trash-2", 16.0, ShellDeckColors::error()))
                .child(
                    div()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("support.delete.title").to_string()),
                ),
        )
        .content(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .px(px(16.0))
                .py(px(16.0))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(ShellDeckColors::text_primary())
                        .child(body_line),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("support.delete.irreversible").to_string()),
                ),
        )
        .footer(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.0))
                .px(px(16.0))
                .py(px(12.0))
                .child(
                    Button::new(cancel_id, t!("support.delete.cancel").to_string())
                        .variant(ButtonVariant::Ghost)
                        .on_click(move |_, _, cx| cancel_close(cx)),
                )
                .child(
                    Button::new(confirm_id, t!("support.delete.confirm").to_string())
                        .variant(ButtonVariant::Destructive)
                        .icon(IconSource::from("trash-2"))
                        .on_click(move |_, _, cx| on_confirm(cx)),
                ),
        )
}

pub(crate) fn render_attachment_delete_dialog(
    id_prefix: &'static str,
    on_close: impl Fn(&mut App) + Clone + 'static,
    on_confirm: impl Fn(&mut App) + Clone + 'static,
) -> impl IntoElement {
    let backdrop_close = on_close.clone();
    let cancel_close = on_close;
    let cancel_id: SharedString = format!("{id_prefix}-cancel").into();
    let confirm_id: SharedString = format!("{id_prefix}-confirm").into();

    UiDialog::new()
        .width(gpui::px(400.0))
        .on_backdrop_click(move |_, cx| backdrop_close(cx))
        .header(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(16.0))
                .py(px(14.0))
                .child(lucide_icon("trash-2", 16.0, ShellDeckColors::error()))
                .child(
                    div()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("user.requests.attachments.delete.title").to_string()),
                ),
        )
        .content(
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .px(px(16.0))
                .py(px(16.0))
                .child(
                    div()
                        .text_size(px(13.0))
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("user.requests.attachments.delete.body").to_string()),
                )
                .child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("user.requests.attachments.delete.irreversible").to_string()),
                ),
        )
        .footer(
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap(px(8.0))
                .px(px(16.0))
                .py(px(12.0))
                .child(
                    Button::new(
                        cancel_id,
                        t!("user.requests.attachments.delete.cancel").to_string(),
                    )
                    .variant(ButtonVariant::Ghost)
                    .on_click(move |_, _, cx| cancel_close(cx)),
                )
                .child(
                    Button::new(
                        confirm_id,
                        t!("user.requests.attachments.delete.confirm").to_string(),
                    )
                    .variant(ButtonVariant::Destructive)
                    .icon(IconSource::from("trash-2"))
                    .on_click(move |_, _, cx| on_confirm(cx)),
                ),
        )
}
