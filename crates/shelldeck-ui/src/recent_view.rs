use crate::icons::lucide_icon;
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;
use adabraka_ui::components::input::{Input, InputSize, InputState};
use adabraka_ui::prelude::{
    scrollable_vertical, Badge, BadgeVariant, Button, ButtonSize, ButtonVariant,
};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::config::activity::{ActivityAction, ActivityEntry, ActivityKind};

const KIND_FILTERS: &[ActivityKind] = &[
    ActivityKind::Terminal,
    ActivityKind::Connection,
    ActivityKind::Forward,
    ActivityKind::Script,
    ActivityKind::Support,
    ActivityKind::Issue,
    ActivityKind::Site,
    ActivityKind::Jean,
    ActivityKind::Fleet,
    ActivityKind::Bext,
    ActivityKind::Error,
];

#[derive(Debug, Clone)]
pub enum RecentEvent {
    Open(ActivityEntry),
}

impl EventEmitter<RecentEvent> for RecentView {}

pub struct RecentView {
    entries: Vec<ActivityEntry>,
    search_state: Entity<InputState>,
    search_query: String,
    selected_kind: Option<ActivityKind>,
    focus_handle: FocusHandle,
}

impl RecentView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            entries: Vec::new(),
            search_state: cx.new(InputState::new),
            search_query: String::new(),
            selected_kind: None,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn set_entries(&mut self, entries: Vec<ActivityEntry>) {
        self.entries = entries;
    }

    fn compact_filter_button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
        Button::new(id, label)
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
    }

    fn kind_label(kind: ActivityKind) -> String {
        match kind {
            ActivityKind::Terminal => t!("recent.kind.terminal").to_string(),
            ActivityKind::Connection => t!("recent.kind.connection").to_string(),
            ActivityKind::Forward => t!("recent.kind.forward").to_string(),
            ActivityKind::Script => t!("recent.kind.script").to_string(),
            ActivityKind::Support => t!("recent.kind.support").to_string(),
            ActivityKind::Issue => t!("recent.kind.issue").to_string(),
            ActivityKind::Jean => t!("recent.kind.jean").to_string(),
            ActivityKind::Fleet => t!("recent.kind.fleet").to_string(),
            ActivityKind::Site => t!("recent.kind.site").to_string(),
            ActivityKind::Bext => t!("recent.kind.bext").to_string(),
            ActivityKind::Error => t!("recent.kind.error").to_string(),
        }
    }

    fn kind_icon(kind: ActivityKind) -> &'static str {
        match kind {
            ActivityKind::Terminal => "terminal",
            ActivityKind::Connection => "server",
            ActivityKind::Forward => "arrow-left-right",
            ActivityKind::Script => "scroll-text",
            ActivityKind::Support => "mail",
            ActivityKind::Issue => "inbox",
            ActivityKind::Jean => "cpu",
            ActivityKind::Fleet => "box",
            ActivityKind::Site => "globe",
            ActivityKind::Bext => "cloud",
            ActivityKind::Error => "triangle-alert",
        }
    }

    fn kind_color(kind: ActivityKind) -> Hsla {
        match kind {
            ActivityKind::Terminal | ActivityKind::Connection => ShellDeckColors::success(),
            ActivityKind::Forward | ActivityKind::Site | ActivityKind::Bext => {
                ShellDeckColors::primary()
            }
            ActivityKind::Script | ActivityKind::Jean | ActivityKind::Fleet => {
                ShellDeckColors::warning()
            }
            ActivityKind::Support | ActivityKind::Issue => ShellDeckColors::primary_hover(),
            ActivityKind::Error => ShellDeckColors::error(),
        }
    }

    fn action_label(action: ActivityAction) -> String {
        match action {
            ActivityAction::ConnectConnection => t!("recent.action.connect").to_string(),
            ActivityAction::OpenTerminal => t!("recent.action.resume").to_string(),
            ActivityAction::None => String::new(),
            _ => t!("recent.action.open").to_string(),
        }
    }

    fn matches_filter(&self, entry: &ActivityEntry) -> bool {
        if self.selected_kind.is_some_and(|kind| kind != entry.kind) {
            return false;
        }
        let q = self.search_query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        entry.message.to_lowercase().contains(&q)
            || entry
                .detail
                .as_ref()
                .is_some_and(|d| d.to_lowercase().contains(&q))
            || entry
                .target_label
                .as_ref()
                .is_some_and(|d| d.to_lowercase().contains(&q))
            || entry
                .target_id
                .as_ref()
                .is_some_and(|d| d.to_lowercase().contains(&q))
    }

    fn render_filters(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut row = div()
            .flex()
            .flex_wrap()
            .gap(px(6.0))
            .px(px(24.0))
            .pb(px(12.0));
        let all_selected = self.selected_kind.is_none();
        row = row.child(
            Self::compact_filter_button("recent-filter-all", t!("recent.filter.all").to_string())
                .selected(all_selected)
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.selected_kind = None;
                    cx.notify();
                })),
        );
        for kind in KIND_FILTERS {
            let selected = self.selected_kind == Some(*kind);
            let id = ElementId::from(SharedString::from(format!("recent-filter-{kind:?}")));
            let label = Self::kind_label(*kind);
            let filter_kind = *kind;
            row = row.child(
                Self::compact_filter_button(id, label)
                    .selected(selected)
                    .on_click(cx.listener(move |this, _event, _window, cx| {
                        this.selected_kind = Some(filter_kind);
                        cx.notify();
                    })),
            );
        }
        row
    }

    fn render_entry(&self, entry: &ActivityEntry, cx: &mut Context<Self>) -> impl IntoElement {
        let color = Self::kind_color(entry.kind);
        let icon = Self::kind_icon(entry.kind);
        let at_ms = entry.at.timestamp_millis() as f64;
        let action = entry.action;
        let action_label = Self::action_label(action);
        let event_for_click = entry.clone();

        let mut meta = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .text_size(px(11.0))
            .text_color(ShellDeckColors::text_muted())
            .child(Badge::new(Self::kind_label(entry.kind)).variant(BadgeVariant::Outline))
            .child(crate::i18n::rel_time(at_ms));

        if let Some(label) = entry.target_label.as_ref().filter(|s| !s.trim().is_empty()) {
            meta = meta.child(
                div()
                    .max_w(px(220.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(label.clone()),
            );
        }

        let mut text_col = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .min_w(px(0.0))
            .flex_grow()
            .child(meta)
            .child(
                div()
                    .text_size(px(14.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_primary())
                    .overflow_hidden()
                    .child(entry.message.clone()),
            );

        if let Some(detail) = entry.detail.as_ref().filter(|d| !d.trim().is_empty()) {
            text_col = text_col.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .overflow_hidden()
                    .child(detail.clone()),
            );
        }

        let mut row = div()
            .flex()
            .items_center()
            .gap(px(12.0))
            .w_full()
            .px(px(14.0))
            .py(px(12.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(32.0))
                    .rounded(px(6.0))
                    .bg(color.opacity(0.12))
                    .flex_shrink_0()
                    .child(lucide_icon(icon, 16.0, color)),
            )
            .child(text_col);

        if action != ActivityAction::None {
            row = row.child(
                Button::new(
                    ElementId::from(SharedString::from(format!("recent-open-{}", entry.id))),
                    action_label,
                )
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .on_click(cx.listener(move |_this, _event, _window, cx| {
                    cx.emit(RecentEvent::Open(event_for_click.clone()));
                })),
            );
        }

        row
    }

    fn render_search(&self, cx: &mut Context<Self>) -> impl IntoElement {
        Input::new(&self.search_state)
            .size(InputSize::Sm)
            .placeholder(t!("recent.search_placeholder").to_string())
            .clearable(true)
            .prefix(
                svg()
                    .path("icons/lucide/search.svg")
                    .size(px(13.0))
                    .text_color(ShellDeckColors::text_muted()),
            )
            .on_change({
                let entity = cx.entity();
                move |value, cx| {
                    entity.update(cx, |this, cx| {
                        this.search_query = value.to_string();
                        cx.notify();
                    });
                }
            })
    }
}

impl Render for RecentView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let filtered: Vec<ActivityEntry> = self
            .entries
            .iter()
            .filter(|entry| self.matches_filter(entry))
            .cloned()
            .collect();

        let mut list = div().flex().flex_col().w_full();
        if filtered.is_empty() {
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(8.0))
                    .py(px(48.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(lucide_icon("activity", 28.0, ShellDeckColors::text_muted()))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .child(t!("recent.empty").to_string()),
                    ),
            );
        } else {
            for entry in &filtered {
                list = list.child(self.render_entry(entry, cx));
            }
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .track_focus(&self.focus_handle)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(16.0))
                    .px(px(24.0))
                    .py(px(18.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(lucide_icon(
                                        "activity",
                                        18.0,
                                        ShellDeckColors::text_muted(),
                                    ))
                                    .child(
                                        div()
                                            .text_size(px(18.0))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(ShellDeckColors::text_primary())
                                            .child(t!("recent.title").to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(t!("recent.subtitle").to_string()),
                            ),
                    )
                    .child(
                        div()
                            .w(px(320.0))
                            .max_w(relative(0.45))
                            .child(self.render_search(cx)),
                    ),
            )
            .child(self.render_filters(cx))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_hidden()
                    .px(px(24.0))
                    .pb(px(24.0))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .h_full()
                            .rounded(px(8.0))
                            .border_1()
                            .border_color(ShellDeckColors::border())
                            .overflow_hidden()
                            .bg(ShellDeckColors::bg_surface())
                            .child(scrollable_vertical(list)),
                    ),
            )
    }
}
