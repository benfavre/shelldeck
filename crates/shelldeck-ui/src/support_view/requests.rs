use super::thread::{
    ai_draft_card, attributed_quote, day_separator, delivery_status, human_message,
    human_message_continuation, local_draft, markdown_blocks, message_action, note as thread_note,
    thread_header_picker, thread_picker_option_row, thread_priority_color, thread_status_color,
    timeline_day, timeline_day_label, typing_indicator, HumanMessageMeta, ThreadDeliveryTone,
    ThreadMessageExtras, ThreadNoteKind,
};
use super::*;
use crate::icons::{ai_provider_icon, ai_provider_inline};
use adabraka_ui::prelude::{tooltip, Composer};

#[derive(Clone, Copy)]
enum TimelineGroup {
    Opening,
    Comment(usize),
    Typing(usize),
    AiDraft,
    LocalDraft,
}

fn thread_scroll_to_restore(
    preserve_scroll: bool,
    old_count: usize,
    old_scroll: gpui::ListOffset,
) -> Option<gpui::ListOffset> {
    (preserve_scroll && old_scroll.item_ix < old_count).then_some(old_scroll)
}

impl SupportView {
    pub(super) fn rebuild_issue_thread_cache(&mut self, preserve_scroll: bool) {
        let old_count = self.issue_thread_list.item_count();
        let old_scroll = thread_scroll_to_restore(
            preserve_scroll,
            old_count,
            self.issue_thread_list.logical_scroll_top(),
        );
        let Some(issue) = &self.issue_detail else {
            self.issue_body_blocks.clear();
            self.issue_comment_blocks.clear();
            self.issue_thread_rows.clear();
            self.issue_thread_list.reset(0);
            return;
        };

        self.issue_body_blocks = markdown_blocks(&issue.body);
        self.issue_comment_blocks = issue
            .comments
            .iter()
            .map(|comment| {
                if comment.is_note() {
                    Vec::new()
                } else {
                    markdown_blocks(&comment.body)
                }
            })
            .collect();

        let mut groups = Vec::new();
        if !issue.body.trim().is_empty()
            || !issue.attachments.is_empty()
            || issue.comments.is_empty()
        {
            groups.push((issue.created_at, 0usize, TimelineGroup::Opening));
        }
        groups.extend(
            issue
                .comments
                .iter()
                .enumerate()
                .map(|(index, comment)| (comment.at, index + 1, TimelineGroup::Comment(index))),
        );
        groups.extend(
            issue
                .thread_state
                .typing
                .iter()
                .enumerate()
                .map(|(index, typing)| {
                    (
                        typing.at,
                        issue.comments.len() + index + 1,
                        TimelineGroup::Typing(index),
                    )
                }),
        );
        if self.issue_ai_draft.is_some() {
            let at = issue
                .thread_state
                .suggested_reply
                .as_ref()
                .map(|draft| draft.at)
                .filter(|at| *at > 0.0)
                .unwrap_or_else(|| chrono::Utc::now().timestamp_millis() as f64);
            groups.push((at, usize::MAX - 1, TimelineGroup::AiDraft));
        }
        if let Some(draft) = issue
            .thread_state
            .local_draft
            .as_ref()
            .filter(|draft| !draft.body.trim().is_empty())
        {
            groups.push((draft.at, usize::MAX, TimelineGroup::LocalDraft));
        }
        groups.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

        let mut rows = Vec::new();
        let mut previous_day = None;
        for (at, _, group) in groups {
            let day = timeline_day(at);
            if previous_day.is_some() && day.is_some() && day != previous_day {
                rows.push(IssueThreadRow::Day { at });
            }
            if day.is_some() {
                previous_day = day;
            }

            match group {
                TimelineGroup::Opening => {
                    let count = self.issue_body_blocks.len().max(1);
                    rows.extend((0..count).map(|block| IssueThreadRow::Opening {
                        block,
                        last: block + 1 == count,
                    }));
                }
                TimelineGroup::Comment(comment) => {
                    let count = if issue.comments[comment].is_note() {
                        1
                    } else {
                        self.issue_comment_blocks[comment].len().max(1)
                    };
                    rows.extend((0..count).map(|block| IssueThreadRow::Comment {
                        comment,
                        block,
                        first: block == 0,
                        last: block + 1 == count,
                    }));
                }
                TimelineGroup::Typing(index) => rows.push(IssueThreadRow::Typing { index }),
                TimelineGroup::AiDraft => rows.push(IssueThreadRow::AiDraft),
                TimelineGroup::LocalDraft => rows.push(IssueThreadRow::LocalDraft),
            }
        }
        self.issue_thread_rows = rows;
        self.issue_thread_list.reset(self.issue_thread_rows.len());
        if let Some(scroll) = old_scroll {
            self.issue_thread_list.scroll_to(scroll);
        }
    }

    fn issue_thread_item_count(&self) -> usize {
        self.issue_thread_rows.len()
    }

    pub(super) fn render_requests(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
            .child(support_refresh_button(
                "support-requests-refresh",
                SupportViewEvent::IssuesRefresh,
                cx,
            ));

        let entity = cx.entity();
        // Simple filter bar — mirrors `render_filters` (tickets) exactly:
        // a search row (input + IconButton "filter" + optional count badge)
        // followed by a chips row (`compact_filter_button` with `selected`).
        // See `.agents/ui-components.md` § harmonization — the two surfaces
        // should never drift.
        let search_row = div()
            .flex()
            .items_center()
            .gap(px(6.0))
            .px(px(10.0))
            .pt(px(8.0))
            .pb(px(6.0))
            .child(
                div().flex_1().child(
                    Input::new(&self.issues_search_state)
                        .size(InputSize::Sm)
                        .placeholder(t!("support.issues.search").to_string())
                        .prefix(lucide_icon("search", 12.0, ShellDeckColors::text_muted()))
                        .on_enter({
                            move |value, cx| {
                                let q = value.to_string();
                                entity.update(cx, |this, cx| {
                                    this.issues_filter.q = q;
                                    let filter = this.issues_filter.clone();
                                    cx.emit(SupportViewEvent::IssuesFilterChanged { filter });
                                });
                            }
                        }),
                ),
            )
            .child(self.render_issues_filter_button(cx));

        let mut chips_row = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(4.0))
            .px(px(10.0))
            .pb(px(6.0));
        let entries: &[(&str, &str)] = &[
            ("", "support.issues.filter.all"),
            ("open", "support.issues.filter.open"),
            ("in_progress", "support.issues.filter.in_progress"),
            ("done", "support.issues.filter.done"),
        ];
        for (value, label_key) in entries {
            let active = self.issues_filter.status == *value;
            let value_owned: String = (*value).to_string();
            let entity = cx.entity();
            chips_row = chips_row.child(
                Self::compact_filter_button(
                    ElementId::from(SharedString::from(format!(
                        "iss-sf-{}",
                        if value.is_empty() { "all" } else { value }
                    ))),
                    t!(*label_key).to_string(),
                )
                .variant(ButtonVariant::Outline)
                .selected(active)
                .on_click({
                    let entity = entity.clone();
                    move |_, _, cx| {
                        let value = value_owned.clone();
                        entity.update(cx, |this, cx| {
                            let q = this.issues_search_state.read(cx).content().to_string();
                            this.issues_filter.status = value;
                            this.issues_filter.q = q;
                            let filter = this.issues_filter.clone();
                            cx.emit(SupportViewEvent::IssuesFilterChanged { filter });
                            cx.notify();
                        });
                    }
                }),
            );
        }

        let mut filter_bar = div()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(search_row)
            .child(chips_row);
        if self.advanced_filter_count() > 0 {
            filter_bar = filter_bar.child(self.render_applied_issues_filter_chips(cx));
        }

        // Internal Support and super-admin staff see every in-scope request the server hands
        // back; a non-staff caller only ever files their own, but we still
        // filter defensively to `is_my_issue` in case tenant scope surfaces
        // a peer's request through Support.
        let visible_count = self.visible_issue_count();
        let list = if visible_count == 0 {
            div()
                .id("sup-issues-list-empty")
                .flex_1()
                .child(
                    div()
                        .p(px(16.0))
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("support.empty.requests").to_string()),
                )
                .into_any_element()
        } else {
            uniform_list(
                "sup-issues-list",
                visible_count,
                cx.processor(|this, range: Range<usize>, _window, cx| {
                    let visible_indices = this
                        .issues
                        .iter()
                        .enumerate()
                        .filter(|(_, issue)| this.issues_staff || this.is_my_issue(issue))
                        .map(|(index, _)| index)
                        .collect::<Vec<_>>();
                    range
                        .filter_map(|index| visible_indices.get(index).copied())
                        .filter_map(|index| this.issues.get(index))
                        .map(|issue| this.render_issue_row(issue, cx).into_any_element())
                        .collect::<Vec<_>>()
                }),
            )
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .into_any_element()
        };

        let left = div()
            .w(px(340.0))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .child(header)
            .child(filter_bar)
            .child(list);

        div()
            .flex_1()
            .flex()
            .min_h(px(0.0))
            .child(left)
            .child(self.render_issue_detail(cx))
    }

    pub(super) fn render_issue_row(&self, iss: &Issue, cx: &mut Context<Self>) -> impl IntoElement {
        let id = iss.id.clone();
        let selected = self.issue_selected.as_deref() == Some(iss.id.as_str());
        let title = if iss.title.trim().is_empty() {
            t!("support.issue.no_title").to_string()
        } else {
            crate::external_content::external_title(&iss.title)
        };
        let when = rel_time(iss.updated_at);
        let group_name = SharedString::from(format!("iss-row-{}", iss.id));

        let mut row = div()
            .id(ElementId::from(SharedString::from(format!(
                "iss-{}",
                iss.id
            ))))
            .group(group_name.clone())
            .w_full()
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

        // The title gets the whole first line. It used to share it with the
        // tag icon, a filled status badge, a filled priority badge and the
        // date — roughly 200 of the column's 340px — so a title like
        // `<@U0BETU0RUMS…` was cut before it had said anything.
        //
        // Status moves down to the metadata line as a coloured dot plus its
        // word, which costs a fraction of a filled pill; priority only shows
        // when it is not the default, because "Normale" on every row is noise.
        let status_dot = match iss.status.as_str() {
            "done" | "closed" => ShellDeckColors::success(),
            "in_progress" | "triaging" => ShellDeckColors::warning(),
            "blocked" => ShellDeckColors::error(),
            _ => ShellDeckColors::primary(),
        };
        row = row
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
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
                    // Per-row kebab. Hand-rolled (matches sidebar's
                    // per-connection kebab) because adabraka `IconButton`
                    // derives its element id from the icon name and would
                    // collide across rows. `group_hover` shows it only on
                    // row hover; `stop_propagation` keeps the click from
                    // opening the detail behind the popover.
                    .child({
                        let iid = iss.id.clone();
                        div()
                            .id(ElementId::from(SharedString::from(format!(
                                "iss-kebab-{}",
                                iss.id
                            ))))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .w(px(22.0))
                            .h(px(22.0))
                            .rounded(px(4.0))
                            .cursor_pointer()
                            .text_color(ShellDeckColors::text_muted())
                            .opacity(0.0)
                            .group_hover(group_name.clone(), |el| el.opacity(1.0))
                            .hover(|el| {
                                el.bg(ShellDeckColors::hover_bg())
                                    .text_color(ShellDeckColors::text_primary())
                            })
                            .child(
                                svg()
                                    .path(lucide_path("ellipsis-vertical"))
                                    .size(px(14.0))
                                    .text_color(ShellDeckColors::text_muted()),
                            )
                            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                                cx.stop_propagation();
                                this.issue_popover_menu = Some((iid.clone(), event.position()));
                                cx.notify();
                            }))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .w_full()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(10.5))
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .flex_shrink_0()
                            .child(div().w(px(6.0)).h(px(6.0)).rounded_full().bg(status_dot))
                            .child(issue_status_label(&iss.status)),
                    )
                    .when(
                        iss.priority != "normal" && !iss.priority.trim().is_empty(),
                        |el| el.child(div().flex_shrink_0().child(priority_badge(&iss.priority))),
                    )
                    .child(div().flex_1().min_w(px(0.0)).truncate().child(meta))
                    .when(!when.is_empty(), |el| {
                        el.child(div().flex_shrink_0().child(when.clone()))
                    }),
            );
        row
    }

    pub(super) fn render_empty_issue_detail(&self) -> Div {
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

    fn render_issue_comment_segment(
        &self,
        c: &shelldeck_core::config::issues::IssueComment,
        body: SharedString,
        first: bool,
        last: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let author_matches_me = !c.author.trim().is_empty() && {
            let a = c.author.trim().to_ascii_lowercase();
            (!self.account_name_lc.is_empty() && a == self.account_name_lc)
                || (!self.account_email_lc.is_empty() && a == self.account_email_lc)
        };
        let label = if c.author.trim().is_empty() {
            t!("support.issue.comment").to_string()
        } else {
            c.author.clone()
        };

        let channel = if c.channel.trim().is_empty() {
            self.issue_detail
                .as_ref()
                .and_then(|iss| Self::source_chip_label(&iss.source))
                .map(SharedString::from)
        } else {
            Some(SharedString::from(c.channel.clone()))
        };
        let attachments = (last && !c.attachments.is_empty())
            .then(|| self.render_issue_attachment_links(&c.attachments, cx));
        let group = SharedString::from(format!("issue-message-{}", c.id));
        let extras = ThreadMessageExtras {
            quote: first
                .then(|| {
                    c.quote
                        .as_ref()
                        .map(|quote| attributed_quote(quote.author.clone(), quote.body.clone()))
                })
                .flatten(),
            delivery: last.then(|| self.render_issue_delivery(c, cx)).flatten(),
            actions: first.then(|| self.render_issue_message_actions(c, cx)),
            group: Some(group),
            link_handler: Some(Self::thread_link_handler(cx)),
        };
        let font_size = px(12.5).to_pixels(window.rem_size());
        if first {
            human_message(
                HumanMessageMeta {
                    author: label.into(),
                    mine: author_matches_me,
                    at: c.at,
                    channel,
                },
                body,
                attachments,
                extras,
                font_size,
            )
        } else {
            human_message_continuation(body, attachments, extras, font_size)
        }
    }

    fn render_issue_message_actions(
        &self,
        comment: &shelldeck_core::config::issues::IssueComment,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        let reply_entity = entity.clone();
        let ai_entity = entity.clone();
        let focus = self.composer_state.read(cx).focus_handle(cx);
        let author = comment.author.clone();
        let quoted = comment.body.clone();
        let reply_id = SharedString::from(format!("issue-reply-{}", comment.id));
        let copy_id = SharedString::from(format!("issue-copy-{}", comment.id));
        let ai_id = SharedString::from(format!("issue-ai-rewrite-{}", comment.id));
        let issue_id = self.issue_selected.clone().unwrap_or_default();
        div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .child(message_action(
                reply_id,
                "reply",
                t!("support.thread.reply").to_string(),
                move |_, window, cx| {
                    let author = author.clone();
                    let quoted = quoted.clone();
                    reply_entity.update(cx, |this, cx| {
                        let current = this.composer_state.read(cx).content().to_string();
                        let prefix = format!("> {} : {}\n\n", author, quoted);
                        let next = if current.trim().is_empty() {
                            prefix
                        } else {
                            format!("{}{}", prefix, current)
                        };
                        this.composer_state
                            .update(cx, |state, cx| state.replace_content(next, cx));
                    });
                    window.focus(&focus);
                },
            ))
            .child(message_action(
                copy_id,
                "copy",
                t!("support.thread.copy").to_string(),
                {
                    let body = comment.body.clone();
                    move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(body.clone()));
                    }
                },
            ))
            .when(self.ai_issue_enabled, |actions| {
                actions.child(message_action(
                    ai_id,
                    "sparkles",
                    t!("support.thread.rewrite_ai").to_string(),
                    move |_, _, cx| {
                        ai_entity.update(cx, |_, cx| {
                            cx.emit(SupportViewEvent::SuggestIssueReply(issue_id.clone()));
                        });
                    },
                ))
            })
            .into_any_element()
    }

    fn render_issue_delivery(
        &self,
        comment: &shelldeck_core::config::issues::IssueComment,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let delivery = comment.delivery.as_ref()?;
        let channel = if delivery.channel.trim().is_empty() {
            if comment.channel.trim().is_empty() {
                "support"
            } else {
                comment.channel.as_str()
            }
        } else {
            delivery.channel.as_str()
        };
        if delivery.status == "failed" {
            let entity = cx.entity();
            let issue_id = self.issue_selected.clone().unwrap_or_default();
            let comment_id = comment.id.clone();
            let retry = message_action(
                SharedString::from(format!("issue-retry-{}", comment.id)),
                "rotate-ccw",
                t!("support.thread.retry").to_string(),
                move |_, _, cx| {
                    entity.update(cx, |_, cx| {
                        cx.emit(SupportViewEvent::RetryIssueComment {
                            issue_id: issue_id.clone(),
                            comment_id: comment_id.clone(),
                        });
                    });
                },
            );
            let label = if delivery.error.trim().is_empty() {
                t!("support.thread.send_failed").to_string()
            } else {
                delivery.error.clone()
            };
            Some(delivery_status(
                label,
                ThreadDeliveryTone::Error,
                Some(retry),
            ))
        } else {
            let label = if delivery.status == "read" && delivery.at > 0.0 {
                t!(
                    "support.thread.sent_read",
                    channel = channel,
                    when = rel_time(delivery.at)
                )
                .to_string()
            } else {
                t!("support.thread.sent", channel = channel).to_string()
            };
            Some(delivery_status(label, ThreadDeliveryTone::Success, None))
        }
    }

    fn render_issue_thread_item(
        &self,
        index: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let final_item = index + 1 == self.issue_thread_item_count();
        let object_bottom = if final_item { 18.0 } else { 20.0 };
        let Some(issue) = &self.issue_detail else {
            return div().into_any_element();
        };
        let Some(row) = self.issue_thread_rows.get(index).cloned() else {
            return div().into_any_element();
        };
        let font_size = px(12.5).to_pixels(window.rem_size());
        let (content, bottom) = match row {
            IssueThreadRow::Opening { block, last } => {
                let body = self
                    .issue_body_blocks
                    .get(block)
                    .cloned()
                    .unwrap_or_else(|| SharedString::from(issue.body.clone()));
                let attachments = (last && !issue.attachments.is_empty())
                    .then(|| self.render_issue_attachment_links(&issue.attachments, cx));
                let content = if block == 0 {
                    human_message(
                        HumanMessageMeta {
                            author: if issue.requested_by.trim().is_empty() {
                                t!("support.issue.description").to_string().into()
                            } else {
                                issue.requested_by.clone().into()
                            },
                            mine: false,
                            at: issue.created_at,
                            channel: Self::source_chip_label(&issue.source).map(SharedString::from),
                        },
                        body,
                        attachments,
                        ThreadMessageExtras {
                            link_handler: Some(Self::thread_link_handler(cx)),
                            ..Default::default()
                        },
                        font_size,
                    )
                } else {
                    human_message_continuation(
                        body,
                        attachments,
                        ThreadMessageExtras {
                            link_handler: Some(Self::thread_link_handler(cx)),
                            ..Default::default()
                        },
                        font_size,
                    )
                };
                (content, if last { object_bottom } else { 8.0 })
            }
            IssueThreadRow::Comment {
                comment,
                block,
                first,
                last,
            } => {
                let comment_index = comment;
                let comment = &issue.comments[comment_index];
                if comment.is_note() {
                    (
                        self.render_issue_note(comment, window).into_any_element(),
                        object_bottom,
                    )
                } else {
                    let body = self.issue_comment_blocks[comment_index]
                        .get(block)
                        .cloned()
                        .unwrap_or_else(|| SharedString::from(comment.body.clone()));
                    (
                        self.render_issue_comment_segment(comment, body, first, last, window, cx),
                        if last { object_bottom } else { 8.0 },
                    )
                }
            }
            IssueThreadRow::Day { at } => (day_separator(timeline_day_label(at)), 14.0),
            IssueThreadRow::Typing { index } => {
                let author = issue
                    .thread_state
                    .typing
                    .get(index)
                    .map(|typing| typing.author.clone())
                    .unwrap_or_default();
                (typing_indicator(author), object_bottom)
            }
            IssueThreadRow::AiDraft => {
                let Some(draft) = &self.issue_ai_draft else {
                    return div().into_any_element();
                };
                (
                    self.render_issue_ai_draft_card(draft.body.clone(), draft.model.clone(), cx)
                        .into_any_element(),
                    object_bottom,
                )
            }
            IssueThreadRow::LocalDraft => {
                let body = issue
                    .thread_state
                    .local_draft
                    .as_ref()
                    .map(|draft| draft.body.clone())
                    .unwrap_or_default();
                (local_draft(body), object_bottom)
            }
        };
        div()
            .w_full()
            .pb(px(bottom))
            .child(content)
            .into_any_element()
    }

    /// One system note (`status`, `system` or `github`), rendered as the
    /// mockup's `.thr-note`: icon + one-line body + actor/time, in the frame
    /// colour that matches the kind.
    fn render_issue_note(
        &self,
        c: &shelldeck_core::config::issues::IssueComment,
        window: &Window,
    ) -> impl IntoElement {
        let kind = match c.kind.as_str() {
            "status" => ThreadNoteKind::Status,
            "github" => ThreadNoteKind::Github,
            // The current wire schema has no separate dispatch kind. Detect
            // the server-authored dispatch wording so the existing event gets
            // the green runtime treatment from the prototype.
            "system" if c.body.trim_start().starts_with("Dispatch") => ThreadNoteKind::Dispatch,
            _ => ThreadNoteKind::System,
        };
        let actor = if c.author.trim().is_empty() {
            None
        } else {
            Some(c.author.clone())
        };
        thread_note(
            c.body.clone(),
            actor,
            c.at,
            kind,
            px(11.5).to_pixels(window.rem_size()),
        )
    }

    /// Human-readable chip for `iss.source`. Returns `None` for values that do
    /// not carry an origin story (or that are the app itself).
    fn source_chip_label(source: &str) -> Option<&'static str> {
        match source {
            "slack" => Some("slack"),
            "github" => Some("github"),
            "manage" => Some("manage"),
            "email" => Some("email"),
            _ => None,
        }
    }

    pub(super) fn render_issue_attachment_links(
        &self,
        attachments: &[IssueAttachment],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity().downgrade();
        let lightbox_attachments = attachments.to_vec();
        let delete_target = if self.section == SupportSection::Requests {
            self.issue_detail
                .as_ref()
                .map(|issue| (true, issue.id.clone()))
        } else {
            self.detail
                .as_ref()
                .map(|ticket| (false, ticket.id.clone()))
        };
        let delete_entity = entity.clone();
        let delete_attachments = attachments.to_vec();
        let on_delete = delete_target.map(|(is_issue, target_id)| {
            Rc::new(move |index: usize, cx: &mut App| {
                let Some(attachment) = delete_attachments.get(index) else {
                    return;
                };
                let target = if is_issue {
                    AttachmentDeleteTarget::Issue {
                        target_id: target_id.clone(),
                        attachment_id: attachment.id.clone(),
                    }
                } else {
                    AttachmentDeleteTarget::Ticket {
                        target_id: target_id.clone(),
                        attachment_id: attachment.id.clone(),
                    }
                };
                if let Some(entity) = delete_entity.upgrade() {
                    entity.update(cx, |this, cx| {
                        this.confirm_attachment_delete = Some(target);
                        cx.notify();
                    });
                }
            }) as Rc<dyn Fn(usize, &mut App)>
        });
        render_stored_attachment_gallery(
            attachments,
            "support-attachment",
            move |index, cx| {
                let Some(entity) = entity.upgrade() else {
                    return;
                };
                let close_entity = entity.downgrade();
                let attachments = lightbox_attachments.clone();
                let lightbox = cx.new(|cx| {
                    AttachmentLightbox::new(
                        attachments,
                        index,
                        move |cx| {
                            if let Some(entity) = close_entity.upgrade() {
                                entity.update(cx, |this, cx| {
                                    this.attachment_lightbox = None;
                                    cx.notify();
                                });
                            }
                        },
                        cx,
                    )
                });
                entity.update(cx, |this, cx| {
                    this.attachment_lightbox = Some(lightbox);
                    cx.notify();
                });
            },
            on_delete,
        )
    }

    pub(super) fn close_issue_popover_menu(&mut self, cx: &mut Context<Self>) {
        self.issue_popover_menu = None;
        cx.notify();
    }

    pub(super) fn build_issue_menu_items(
        &self,
        iss: &Issue,
        entity: Entity<SupportView>,
    ) -> Vec<PopoverMenuItem> {
        let id = iss.id.clone();
        let mut items = Vec::new();

        // The prototype keeps only two quiet header actions. Triage remains
        // available here instead of occupying a third outlined icon button.
        if self.ai_issue_enabled && self.issues_staff {
            let triage_id = id.clone();
            items.push(
                PopoverMenuItem::new(
                    "iss-menu-ai-triage",
                    t!("ai.workflow.issue_triage").to_string(),
                )
                .icon("sparkles")
                .on_click({
                    let entity = entity.clone();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.close_issue_popover_menu(cx);
                            cx.emit(SupportViewEvent::TriageIssue(triage_id.clone()));
                        });
                    }
                }),
            );
        }

        if self.issues_staff {
            items.extend(Self::staff_secondary_items(
                iss,
                &id,
                &self.issue_instances,
                &entity,
            ));
        }

        // Delete: staff can delete any in-scope request; a non-staff caller
        // only sees the entry on requests they filed themselves. The server
        // enforces the same rule on the wire — this is UX politeness, not
        // security.
        if self.issues_staff || self.is_my_issue(iss) {
            let did = id.clone();
            items.push(
                PopoverMenuItem::new("iss-menu-delete", t!("support.menu.delete").to_string())
                    .icon("trash-2")
                    .on_click({
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_issue_popover_menu(cx);
                                this.confirm_issue_delete = Some(did.clone());
                                cx.notify();
                            });
                        }
                    }),
            );
        }

        items
    }

    /// Staff-only secondary actions for the issue overflow menu. Status,
    /// priority and assignment intentionally do not appear here: their values
    /// are the direct selectors in the header. Each available runtime gets a
    /// concrete dispatch entry so choosing it never opens an inline panel that
    /// changes the header height.
    pub(super) fn staff_secondary_items(
        iss: &Issue,
        id: &str,
        instances: &[IssueInstance],
        entity: &Entity<SupportView>,
    ) -> Vec<PopoverMenuItem> {
        let mut items = Vec::new();
        for instance in instances {
            let dispatch_id = id.to_string();
            let instance_id = instance.id.clone();
            let duplicate_name = instances
                .iter()
                .filter(|candidate| candidate.name.eq_ignore_ascii_case(&instance.name))
                .count()
                > 1;
            let target = if duplicate_name {
                let short_id = instance.id.chars().take(8).collect::<String>();
                format!("{} · {short_id}", instance.name)
            } else {
                instance.name.clone()
            };
            let label = t!("support.menu.dispatch_to", name = target).to_string();
            items.push(
                PopoverMenuItem::new(
                    SharedString::from(format!("iss-menu-dispatch-{}", instance.id)),
                    label,
                )
                .icon("server")
                .on_click({
                    let entity = entity.clone();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.close_issue_popover_menu(cx);
                            cx.emit(SupportViewEvent::IssueDispatch {
                                id: dispatch_id.clone(),
                                instance_id: instance_id.clone(),
                            });
                        });
                    }
                }),
            );
        }

        let gid = id.to_string();
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
                PopoverMenuItem::new(
                    "iss-menu-gh-push",
                    t!("support.menu.github_create").to_string(),
                )
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

    pub(super) fn close_delete_issue_modal(&mut self, cx: &mut Context<Self>) {
        self.confirm_issue_delete = None;
        cx.notify();
    }

    /// Small chip helper used inside the advanced filter modal. Wraps
    /// `compact_filter_button` (the same building block the tickets modal
    /// uses via `render_pick_button`) with `selected(active)` + optional
    /// leading icon — same visual language across both surfaces per
    /// `.agents/ui-components.md` § harmonization.
    pub(super) fn render_filter_chip<F>(
        &self,
        id: SharedString,
        icon: Option<&'static str>,
        label: String,
        is_active: bool,
        cx: &mut Context<Self>,
        on_pick: F,
    ) -> impl IntoElement
    where
        F: Fn(&mut Self) + 'static,
    {
        let entity = cx.entity();
        let mut btn = Self::compact_filter_button(ElementId::from(id), label)
            .variant(ButtonVariant::Outline)
            .selected(is_active);
        if let Some(slug) = icon {
            btn = btn.icon(IconSource::from(slug));
        }
        btn.on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| {
                on_pick(this);
                cx.notify();
            });
        })
    }

    pub(super) fn render_issues_filter_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let d = &self.issues_filter_draft;

        // Priority — colored chips (Destructive / Warning / Secondary /
        // Outline via `priority_badge`) wrapped in a bordered pill that
        // highlights on active. Mirrors the tickets modal priority row so
        // the visual language of "priority" is identical across surfaces.
        let priority_entries: &[(&str, &str)] = &[
            ("", "support.issues.filter.all"),
            ("low", "support.issues.priority.low"),
            ("normal", "support.issues.priority.normal"),
            ("high", "support.issues.priority.high"),
            ("urgent", "support.issues.priority.urgent"),
        ];
        let mut priority_row = div().flex().items_center().gap(px(6.0)).flex_wrap();
        for (value, key) in priority_entries {
            let value: String = (*value).to_string();
            let active = d.priority == value;
            let entity = cx.entity();
            let value_click = value.clone();
            let mut chip = div()
                .id(ElementId::from(SharedString::from(format!(
                    "iss-adv-pri-{}",
                    if value.is_empty() { "all" } else { &value }
                ))))
                .p(px(2.0))
                .rounded_full()
                .cursor_pointer()
                .border_2()
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| {
                        this.issues_filter_draft.priority = value_click.clone();
                        cx.notify();
                    });
                });
            chip = if value.is_empty() {
                chip.child(Badge::new(t!(*key).to_string()).variant(BadgeVariant::Outline))
            } else {
                chip.child(priority_badge(&value))
            };
            chip = if active {
                chip.border_color(ShellDeckColors::primary())
            } else {
                chip.border_color(gpui::transparent_black()).opacity(0.55)
            };
            priority_row = priority_row.child(chip);
        }

        // Source
        let source_entries: &[(&str, &str, &'static str)] = &[
            ("", "support.issues.filter.all", "ellipsis"),
            ("user", "support.issues.source.user", "user"),
            ("support", "support.issues.source.support", "reply"),
        ];
        let mut source_row = div().flex().items_center().gap(px(4.0)).flex_wrap();
        for (value, key, icon) in source_entries {
            let value: String = (*value).to_string();
            let active = d.source == value;
            source_row = source_row.child(self.render_filter_chip(
                SharedString::from(format!(
                    "iss-adv-src-{}",
                    if value.is_empty() { "all" } else { &value }
                )),
                Some(icon),
                t!(*key).to_string(),
                active,
                cx,
                move |this| this.issues_filter_draft.source = value.clone(),
            ));
        }

        // Assignee — a click-to-open button that pops a full modal picker
        // (search + scrollable list). Selects work fine for small option
        // sets but agents can grow, and the picker overlay reads better
        // than a cramped popover — matches the pattern the sidebar / site
        // switcher use for "search then pick".
        let assignee_label = self.issues_assignee_label(&d.assignee);
        let assignee_row = {
            let entity = cx.entity();
            div()
                .id("iss-adv-as-open")
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .w_full()
                .px(px(10.0))
                .py(px(6.0))
                .rounded(px(6.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_primary())
                .text_size(px(12.0))
                .text_color(ShellDeckColors::text_primary())
                .cursor_pointer()
                .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(lucide_icon(
                            "user-check",
                            12.0,
                            ShellDeckColors::text_muted(),
                        ))
                        .child(assignee_label),
                )
                .child(lucide_icon(
                    "chevron-down",
                    12.0,
                    ShellDeckColors::text_muted(),
                ))
                .on_click(move |_, _, cx| {
                    entity.update(cx, |this, cx| this.open_issues_assignee_modal(cx));
                })
        };

        // GitHub linkage — 3 chips (all / linked / not linked)
        let github_entries: &[(Option<bool>, &str, &str, &'static str)] = &[
            (None, "all", "support.issues.filter.all", "ellipsis"),
            (
                Some(true),
                "linked",
                "support.issues.github.linked",
                "upload",
            ),
            (
                Some(false),
                "unlinked",
                "support.issues.github.unlinked",
                "eye-off",
            ),
        ];
        let mut github_row = div().flex().items_center().gap(px(4.0)).flex_wrap();
        for (value, tag, key, icon) in github_entries {
            let value_c = *value;
            let active = d.has_github == value_c;
            github_row = github_row.child(self.render_filter_chip(
                SharedString::from(format!("iss-adv-gh-{}", tag)),
                Some(icon),
                t!(*key).to_string(),
                active,
                cx,
                move |this| this.issues_filter_draft.has_github = value_c,
            ));
        }

        // Since — 4 chips: all / 24h / 7d / 30d. We stamp an ISO instant at
        // pick time — server does lexicographic compare on ISO strings, so
        // `now - offset` in UTC formatted as `%Y-%m-%dT%H:%M:%SZ` is enough.
        fn iso_since(hours: i64) -> String {
            let then = chrono::Utc::now() - chrono::Duration::hours(hours);
            then.format("%Y-%m-%dT%H:%M:%SZ").to_string()
        }
        let since_entries: &[(&str, &str, Option<i64>, &'static str)] = &[
            ("all", "support.issues.since.all", None, "ellipsis"),
            ("24h", "support.issues.since.h24", Some(24), "clock"),
            ("7d", "support.issues.since.d7", Some(24 * 7), "clock"),
            ("30d", "support.issues.since.d30", Some(24 * 30), "calendar"),
        ];
        let mut since_row = div().flex().items_center().gap(px(4.0)).flex_wrap();
        for (tag, key, hours, icon) in since_entries {
            let hours = *hours;
            let active = match hours {
                None => d.since.is_empty(),
                Some(_) => !d.since.is_empty(),
            };
            since_row = since_row.child(self.render_filter_chip(
                SharedString::from(format!("iss-adv-sc-{}", tag)),
                Some(icon),
                t!(*key).to_string(),
                active,
                cx,
                move |this| {
                    this.issues_filter_draft.since = match hours {
                        None => String::new(),
                        Some(h) => iso_since(h),
                    };
                },
            ));
        }

        let mine_toggle = Checkbox::new("iss-adv-mine")
            .checked(d.mine)
            .label(t!("support.issues.mine").to_string())
            .on_click({
                let entity = entity.clone();
                move |checked, _, cx| {
                    let val = *checked;
                    entity.update(cx, |this, cx| {
                        this.issues_filter_draft.mine = val;
                        cx.notify();
                    });
                }
            });

        UiDialog::new()
            .width(gpui::px(420.0))
            .on_backdrop_click({
                let entity = entity.clone();
                move |_, cx| {
                    entity.update(cx, |this, cx| this.close_issues_filter_modal(cx));
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
                            .child(lucide_icon("filter", 14.0, ShellDeckColors::text_primary()))
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
                                    "iss-filter-reset",
                                    t!("support.filters.reset").to_string(),
                                )
                                .variant(ButtonVariant::Ghost)
                                .on_click({
                                    let entity = entity.clone();
                                    move |_, _, cx| {
                                        entity.update(cx, |this, cx| {
                                            this.reset_issues_filter_draft(cx);
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
                                                this.close_issues_filter_modal(cx);
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
                    .gap(px(14.0))
                    .px(px(16.0))
                    .py(px(12.0))
                    .child(Label::new(t!("support.issues.priority.title").to_string()))
                    .child(priority_row)
                    .child(Label::new(t!("support.issues.source.title").to_string()))
                    .child(source_row)
                    .child(Label::new(t!("support.issues.assignee.title").to_string()))
                    .child(assignee_row)
                    .child(Label::new(t!("support.issues.github.title").to_string()))
                    .child(github_row)
                    .child(Label::new(t!("support.issues.since.title").to_string()))
                    .child(since_row)
                    .child(mine_toggle),
            )
            .footer(
                div()
                    .px(px(14.0))
                    .py(px(12.0))
                    .border_t_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        Self::compact_filter_button(
                            "iss-filter-apply",
                            t!("support.filters.apply").to_string(),
                        )
                        .variant(ButtonVariant::Default)
                        .icon(IconSource::from("check"))
                        .w_full()
                        .on_click({
                            let entity = entity.clone();
                            move |_, _, cx| {
                                entity.update(cx, |this, cx| this.apply_issues_filter_draft(cx));
                            }
                        }),
                    ),
            )
    }

    /// Full-modal assignee picker for the issues advanced filter — opens
    /// on top of the filter modal. Search input + scrollable list of
    /// three specials + the shared agent roster (`self.agents`). Reads
    /// the search query live from the input state at render time — same
    /// pattern as the sites-search input in the User home (adabraka
    /// `on_change` doesn't fire on typing, only programmatic set_value).
    pub(super) fn render_issues_assignee_picker_modal(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let current = self.issues_filter_draft.assignee.clone();
        let query = self
            .issues_assignee_search_state
            .read(cx)
            .content()
            .trim()
            .to_lowercase();

        // Build the option list: specials always first (never filtered),
        // then agents matching the search.
        struct Option {
            value: String,
            label: String,
            subtitle: String,
            icon: &'static str,
        }
        let mut options: Vec<Option> = Vec::new();
        options.push(Option {
            value: String::new(),
            label: t!("support.issues.assignee.all").to_string(),
            subtitle: t!("support.issues.assignee.hint.all").to_string(),
            icon: "users",
        });
        options.push(Option {
            value: "me".to_string(),
            label: t!("support.issues.assignee.me").to_string(),
            subtitle: t!("support.issues.assignee.hint.me").to_string(),
            icon: "user-check",
        });
        options.push(Option {
            value: "unassigned".to_string(),
            label: t!("support.issues.assignee.unassigned").to_string(),
            subtitle: t!("support.issues.assignee.hint.unassigned").to_string(),
            icon: "user",
        });
        for agent in &self.agents {
            let label = if agent.name.trim().is_empty() {
                agent.email.clone()
            } else {
                agent.name.clone()
            };
            let subtitle = agent.email.clone();
            let matches = query.is_empty()
                || label.to_lowercase().contains(&query)
                || subtitle.to_lowercase().contains(&query);
            if !matches {
                continue;
            }
            options.push(Option {
                value: agent.email.clone(),
                label,
                subtitle,
                icon: "user-check",
            });
        }
        let has_agents_match = options.len() > 3;

        // List body — one row per option, click applies + closes.
        let mut list = div().flex().flex_col().gap(px(2.0));
        for opt in options {
            let is_active = opt.value == current;
            let value = opt.value.clone();
            let row = div()
                .id(ElementId::from(SharedString::from(format!(
                    "iss-as-pick-{}",
                    if value.is_empty() {
                        "all".to_string()
                    } else {
                        value.clone()
                    }
                ))))
                .flex()
                .items_center()
                .gap(px(10.0))
                .px(px(10.0))
                .py(px(8.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|s| s.bg(ShellDeckColors::hover_bg()));
            let row = if is_active {
                row.bg(ShellDeckColors::primary().opacity(0.10))
            } else {
                row
            };
            list = list.child(
                row.child(lucide_icon(
                    opt.icon,
                    14.0,
                    if is_active {
                        ShellDeckColors::primary()
                    } else {
                        ShellDeckColors::text_muted()
                    },
                ))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(ShellDeckColors::text_primary())
                                .truncate()
                                .child(opt.label.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::text_muted())
                                .truncate()
                                .child(opt.subtitle.clone()),
                        ),
                )
                .when(is_active, |el| {
                    el.child(lucide_icon("check", 14.0, ShellDeckColors::primary()))
                })
                .on_click({
                    let entity = entity.clone();
                    move |_, _, cx| {
                        let value = value.clone();
                        entity.update(cx, |this, cx| this.pick_issues_assignee(value, cx));
                    }
                }),
            );
        }

        // Empty state if the search filters everything out (specials
        // always show, but if there are no agents matching we say so).
        if !has_agents_match && !query.is_empty() {
            list = list.child(
                div()
                    .px(px(10.0))
                    .py(px(8.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.issues.assignee.no_match").to_string()),
            );
        }

        UiDialog::new()
            .width(gpui::px(420.0))
            .on_backdrop_click({
                let entity = entity.clone();
                move |_, cx| {
                    entity.update(cx, |this, cx| this.close_issues_assignee_modal(cx));
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
                                "user-check",
                                14.0,
                                ShellDeckColors::text_primary(),
                            ))
                            .child(
                                div()
                                    .text_size(px(15.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(t!("support.issues.assignee.picker.title").to_string()),
                            ),
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
                                        this.close_issues_assignee_modal(cx);
                                    });
                                }
                            }),
                    ),
            )
            .content(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.0))
                    .px(px(14.0))
                    .py(px(12.0))
                    .child(
                        Input::new(&self.issues_assignee_search_state)
                            .size(InputSize::Sm)
                            .placeholder(t!("support.issues.assignee.picker.search").to_string())
                            .prefix(lucide_icon("search", 12.0, ShellDeckColors::text_muted()))
                            .on_change({
                                let entity = entity.clone();
                                move |_, cx| {
                                    entity.update(cx, |_, cx| cx.notify());
                                }
                            }),
                    )
                    .child(
                        div()
                            .max_h(px(340.0))
                            .id("iss-as-pick-list")
                            .overflow_y_scroll()
                            .child(list),
                    ),
            )
    }

    /// Destructive confirm modal for a request soft-delete (staff only).
    pub(super) fn render_delete_issue_modal(
        &self,
        id: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let title: SharedString = crate::external_content::external_title(
            &self
                .issue_detail
                .as_ref()
                .filter(|i| i.id == id)
                .map(|i| i.title.clone())
                .or_else(|| {
                    self.issues
                        .iter()
                        .find(|i| i.id == id)
                        .map(|i| i.title.clone())
                })
                .unwrap_or_default(),
        )
        .into();

        let close_entity = entity.clone();
        let confirm_entity = entity;
        let confirm_id = id;

        render_issue_delete_dialog(
            title,
            "iss-del",
            move |cx| {
                close_entity.update(cx, |this, cx| this.close_delete_issue_modal(cx));
            },
            move |cx| {
                let id = confirm_id.clone();
                confirm_entity.update(cx, |this, cx| {
                    this.confirm_issue_delete = None;
                    cx.emit(SupportViewEvent::IssueDelete(id));
                    cx.notify();
                });
            },
        )
    }

    pub(super) fn render_delete_attachment_modal(
        &self,
        target: AttachmentDeleteTarget,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let close_entity = entity.clone();
        let confirm_entity = entity;
        render_attachment_delete_dialog(
            "support-attachment-delete",
            move |cx| {
                close_entity.update(cx, |this, cx| {
                    this.confirm_attachment_delete = None;
                    cx.notify();
                });
            },
            move |cx| {
                let target = target.clone();
                confirm_entity.update(cx, |this, cx| {
                    this.confirm_attachment_delete = None;
                    match target {
                        AttachmentDeleteTarget::Ticket {
                            target_id,
                            attachment_id,
                        } => cx.emit(SupportViewEvent::SupportAttachmentDelete {
                            id: target_id,
                            attachment_id,
                        }),
                        AttachmentDeleteTarget::Issue {
                            target_id,
                            attachment_id,
                        } => cx.emit(SupportViewEvent::IssueAttachmentDelete {
                            id: target_id,
                            attachment_id,
                        }),
                    }
                    cx.notify();
                });
            },
        )
    }

    pub(super) fn render_issue_popover(
        &self,
        iss: &Issue,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let items = self.build_issue_menu_items(iss, entity.clone());
        PopoverMenu::new(pos, items)
            .max_height(gpui::px(360.0))
            .on_close({
                let entity = entity.clone();
                move |_, cx| {
                    entity.update(cx, |this, cx| {
                        this.close_issue_popover_menu(cx);
                    });
                }
            })
    }

    fn render_issue_status_picker(&self, iss: &Issue, cx: &mut Context<Self>) -> AnyElement {
        let trigger = thread_header_picker(
            "iss-detail-status",
            div()
                .size(px(6.0))
                .rounded_full()
                .bg(thread_status_color(&iss.status)),
            issue_status_label(&iss.status),
            self.issues_staff,
        );
        if !self.issues_staff {
            return trigger;
        }

        let parent = cx.entity();
        let issue_id = iss.id.clone();
        let current = iss.status.clone();
        Popover::new("iss-detail-status-popover")
            .trigger(trigger)
            .content(move |window, cx| {
                let parent = parent.clone();
                let issue_id = issue_id.clone();
                let current = current.clone();
                cx.new(move |content_cx| {
                    PopoverContent::new(window, content_cx, move |_window, cx| {
                        let mut list = div().w(px(176.0)).flex().flex_col().gap(px(2.0));
                        for status in [
                            "open",
                            "triaging",
                            "in_progress",
                            "blocked",
                            "done",
                            "closed",
                        ] {
                            let row_parent = parent.clone();
                            let row_issue_id = issue_id.clone();
                            let marker = div()
                                .size(px(7.0))
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(thread_status_color(status));
                            list = list.child(
                                thread_picker_option_row(
                                    format!("iss-status-option-{status}").into(),
                                    marker,
                                    issue_status_label(status),
                                    None,
                                    current == status,
                                )
                                .on_click(cx.listener(
                                    move |_content, _: &ClickEvent, _, cx| {
                                        row_parent.update(cx, |_this, cx| {
                                            cx.emit(SupportViewEvent::IssueStatus {
                                                id: row_issue_id.clone(),
                                                status: status.to_string(),
                                            });
                                        });
                                        cx.emit(DismissEvent);
                                    },
                                )),
                            );
                        }
                        list.into_any_element()
                    })
                })
            })
            .into_any_element()
    }

    fn render_issue_priority_picker(&self, iss: &Issue, cx: &mut Context<Self>) -> AnyElement {
        let trigger = thread_header_picker(
            "iss-detail-priority",
            div()
                .size(px(6.0))
                .rounded_full()
                .bg(thread_priority_color(&iss.priority)),
            priority_label(&iss.priority),
            self.issues_staff,
        );
        if !self.issues_staff {
            return trigger;
        }

        let parent = cx.entity();
        let issue_id = iss.id.clone();
        let current = iss.priority.clone();
        Popover::new("iss-detail-priority-popover")
            .trigger(trigger)
            .content(move |window, cx| {
                let parent = parent.clone();
                let issue_id = issue_id.clone();
                let current = current.clone();
                cx.new(move |content_cx| {
                    PopoverContent::new(window, content_cx, move |_window, cx| {
                        let mut list = div().w(px(176.0)).flex().flex_col().gap(px(2.0));
                        for priority in ["low", "normal", "high", "urgent"] {
                            let row_parent = parent.clone();
                            let row_issue_id = issue_id.clone();
                            let marker = div()
                                .size(px(7.0))
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(thread_priority_color(priority));
                            list = list.child(
                                thread_picker_option_row(
                                    format!("iss-priority-option-{priority}").into(),
                                    marker,
                                    priority_label(priority),
                                    None,
                                    current == priority,
                                )
                                .on_click(cx.listener(
                                    move |_content, _: &ClickEvent, _, cx| {
                                        row_parent.update(cx, |_this, cx| {
                                            cx.emit(SupportViewEvent::IssuePriority {
                                                id: row_issue_id.clone(),
                                                priority: priority.to_string(),
                                            });
                                        });
                                        cx.emit(DismissEvent);
                                    },
                                )),
                            );
                        }
                        list.into_any_element()
                    })
                })
            })
            .into_any_element()
    }

    fn render_issue_assignee_picker(&self, iss: &Issue, cx: &mut Context<Self>) -> AnyElement {
        let trigger = thread_header_picker(
            "iss-detail-assignee",
            lucide_icon("at-sign", 11.0, ShellDeckColors::text_muted()),
            self.assignee_label(&iss.assignee),
            self.issues_staff,
        );
        if !self.issues_staff {
            return trigger;
        }

        let mut agents = Vec::<HeaderAssigneeOption>::new();
        for agent in &self.agents {
            if agent.email.trim().is_empty()
                || agents
                    .iter()
                    .any(|known| known.email.eq_ignore_ascii_case(&agent.email))
            {
                continue;
            }
            agents.push(HeaderAssigneeOption {
                value: agent.email.clone(),
                label: if agent.name.trim().is_empty() {
                    agent.email.clone()
                } else {
                    agent.name.clone()
                },
                email: agent.email.clone(),
            });
        }
        agents.sort_by_key(|agent| agent.label.to_lowercase());

        let total = agents.len();
        let parent = cx.entity();
        let issue_id = iss.id.clone();
        let current = iss.assignee.clone();
        let me_name = self.me.name.clone();
        let me_email = self.me.email.clone();
        let search = self.issue_assignee_search_state.clone();
        let search_placeholder = t!("support.issues.assignee.picker.search").to_string();
        let empty_label = t!("support.assignee.none").to_string();
        let me_label = if me_name.trim().is_empty() {
            t!("support.assignee.me").to_string()
        } else {
            format!("{} · {me_name}", t!("support.assignee.me"))
        };

        Popover::new("iss-detail-assignee-popover")
            .trigger(trigger)
            .content(move |window, cx| {
                search.update(cx, InputState::reset);
                let parent = parent.clone();
                let issue_id = issue_id.clone();
                let current = current.clone();
                let me_name = me_name.clone();
                let me_email = me_email.clone();
                let search = search.clone();
                let search_placeholder = search_placeholder.clone();
                let empty_label = empty_label.clone();
                let me_label = me_label.clone();
                let agents = agents.clone();
                cx.new(move |content_cx| {
                    PopoverContent::new(window, content_cx, move |_window, cx| {
                        let query = search.read(cx).content().trim().to_lowercase();
                        let filtered = agents
                            .iter()
                            .filter(|agent| {
                                query.is_empty()
                                    || agent.label.to_lowercase().contains(&query)
                                    || agent.email.to_lowercase().contains(&query)
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        let filtered_count = filtered.len();
                        let count_label = if query.is_empty() {
                            t!("support.issue.assignee_count", count = total).to_string()
                        } else {
                            t!(
                                "support.issue.assignee_count_filtered",
                                filtered = filtered_count,
                                total = total
                            )
                            .to_string()
                        };
                        let list_height = px((filtered_count.clamp(1, 5) as f32) * 40.0);
                        let filtered = Rc::new(filtered);
                        let content_entity = cx.entity();

                        let none_active = current.trim().is_empty();
                        let me_active = current.eq_ignore_ascii_case("me")
                            || (!me_email.trim().is_empty()
                                && current.eq_ignore_ascii_case(&me_email))
                            || (!me_name.trim().is_empty()
                                && current.eq_ignore_ascii_case(&me_name));

                        let none_parent = parent.clone();
                        let none_issue_id = issue_id.clone();
                        let me_parent = parent.clone();
                        let me_issue_id = issue_id.clone();
                        let rows_parent = parent.clone();
                        let rows_issue_id = issue_id.clone();
                        let rows_current = current.clone();
                        let rows = filtered.clone();

                        div()
                            .w(px(320.0))
                            .flex()
                            .flex_col()
                            .gap(px(5.0))
                            .child(
                                Input::new(&search)
                                    .size(InputSize::Sm)
                                    .placeholder(search_placeholder.clone())
                                    .on_change(move |_, cx| {
                                        content_entity.update(cx, |_content, cx| cx.notify());
                                    }),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .child(
                                        thread_picker_option_row(
                                            "iss-assignee-none".into(),
                                            lucide_icon(
                                                "at-sign",
                                                12.0,
                                                ShellDeckColors::text_muted(),
                                            ),
                                            empty_label.clone(),
                                            None,
                                            none_active,
                                        )
                                        .on_click(cx.listener(
                                            move |_content, _: &ClickEvent, _, cx| {
                                                none_parent.update(cx, |_this, cx| {
                                                    cx.emit(SupportViewEvent::IssueAssign {
                                                        id: none_issue_id.clone(),
                                                        assignee: String::new(),
                                                    });
                                                });
                                                cx.emit(DismissEvent);
                                            },
                                        )),
                                    )
                                    .child(
                                        thread_picker_option_row(
                                            "iss-assignee-me".into(),
                                            lucide_icon(
                                                "at-sign",
                                                12.0,
                                                ShellDeckColors::text_muted(),
                                            ),
                                            me_label.clone(),
                                            None,
                                            me_active,
                                        )
                                        .on_click(cx.listener(
                                            move |_content, _: &ClickEvent, _, cx| {
                                                me_parent.update(cx, |_this, cx| {
                                                    cx.emit(SupportViewEvent::IssueAssign {
                                                        id: me_issue_id.clone(),
                                                        assignee: "me".to_string(),
                                                    });
                                                });
                                                cx.emit(DismissEvent);
                                            },
                                        )),
                                    ),
                            )
                            .child(
                                div()
                                    .pt(px(4.0))
                                    .border_t_1()
                                    .border_color(ShellDeckColors::border())
                                    .child(if filtered_count == 0 {
                                        div()
                                            .h(px(40.0))
                                            .flex()
                                            .items_center()
                                            .px(px(8.0))
                                            .text_size(px(11.0))
                                            .text_color(ShellDeckColors::text_muted())
                                            .child(t!("support.issues.assignee.no_match").to_string())
                                            .into_any_element()
                                    } else {
                                        uniform_list(
                                            "issue-header-assignee-options",
                                            filtered_count,
                                            cx.processor(move |_content, range: Range<usize>, _window, cx| {
                                                range
                                                    .filter_map(|index| {
                                                        rows.get(index)
                                                            .cloned()
                                                            .map(|agent| (index, agent))
                                                    })
                                                    .map(|(index, agent)| {
                                                        let active = rows_current.eq_ignore_ascii_case(&agent.value)
                                                            || rows_current.eq_ignore_ascii_case(&agent.label);
                                                        let row_parent = rows_parent.clone();
                                                        let row_issue_id = rows_issue_id.clone();
                                                        let value = agent.value.clone();
                                                        thread_picker_option_row(
                                                            format!("iss-assignee-agent-{index}").into(),
                                                            lucide_icon(
                                                                "at-sign",
                                                                12.0,
                                                                ShellDeckColors::text_muted(),
                                                            ),
                                                            agent.label,
                                                            Some(agent.email.into()),
                                                            active,
                                                        )
                                                        .h(px(40.0))
                                                        .on_click(cx.listener(
                                                            move |_content, _: &ClickEvent, _, cx| {
                                                                row_parent.update(cx, |_this, cx| {
                                                                    cx.emit(SupportViewEvent::IssueAssign {
                                                                        id: row_issue_id.clone(),
                                                                        assignee: value.clone(),
                                                                    });
                                                                });
                                                                cx.emit(DismissEvent);
                                                            },
                                                        ))
                                                        .into_any_element()
                                                    })
                                                    .collect::<Vec<_>>()
                                            }),
                                        )
                                        .h(list_height)
                                        .w_full()
                                        .into_any_element()
                                    }),
                            )
                            .child(
                                div()
                                    .px(px(8.0))
                                    .text_size(px(9.5))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(count_label),
                            )
                            .into_any_element()
                    })
                })
            })
            .into_any_element()
    }

    pub(super) fn render_issue_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(iss) = self.issue_detail.clone() else {
            return self.render_empty_issue_detail().into_any_element();
        };

        // The badges ARE the triggers. The status and priority pickers already
        // existed — buried in the kebab, two clicks away (⋮ → Statut → choose).
        // The thing you want to change is right there on screen; making it the
        // button is the whole point of the mockup's `[• À traiter ⌄]`.
        // Non-staff still get plain badges: no chevron, no click.
        let mut context = Vec::new();
        if !iss.tenant_name.trim().is_empty() {
            context.push(iss.tenant_name.clone());
        }
        if iss.updated_at > 0.0 {
            context.push(t!("support.issue.updated", when = rel_time(iss.updated_at)).to_string());
        }

        let mut meta_row = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(6.0))
            .child(self.render_issue_status_picker(&iss, cx))
            .child(self.render_issue_priority_picker(&iss, cx))
            .child(self.render_issue_assignee_picker(&iss, cx));
        if !context.is_empty() {
            meta_row = meta_row.child(
                div()
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(context.join(" · ")),
            );
        }
        if let Some(label) = iss.site_label.as_ref().filter(|l| !l.trim().is_empty()) {
            meta_row = meta_row.child(Badge::new(label.clone()).variant(BadgeVariant::Outline));
        }
        if let Some(g) = &iss.github {
            meta_row = meta_row.child(
                Badge::new(format!("GitHub #{}", g.number)).variant(BadgeVariant::Secondary),
            );
        }

        let header = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(8.0))
            .px(px(16.0))
            .py(px(10.0))
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
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(crate::external_content::external_title(&iss.title)),
                    )
                    .child({
                        let summary_id = iss.id.clone();
                        tooltip(
                            Button::new(
                                "issue-ai-summary",
                                t!("support.issue.summarize").to_string(),
                            )
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .h(px(28.0))
                            .px(px(8.0))
                            .icon(IconSource::from("sparkles"))
                            .disabled(!self.ai_issue_enabled)
                            .on_click(cx.listener(
                                move |_, _, _, cx| {
                                    cx.emit(SupportViewEvent::SummarizeIssue(summary_id.clone()));
                                },
                            )),
                            t!("ai.workflow.issue_summary").to_string(),
                        )
                    })
                    .child({
                        let entity = cx.entity();
                        let iid = iss.id.clone();
                        // Keep the IconButton itself as the hit target. The
                        // generic tooltip wrapper creates a separate relative
                        // hit-test node here and swallowed the header click.
                        IconButton::new("ellipsis")
                            .variant(ButtonVariant::Ghost)
                            .size(gpui::px(28.0))
                            .icon_size(gpui::px(14.0))
                            .on_click({
                                move |event, _, cx| {
                                    let iid = iid.clone();
                                    entity.update(cx, |this, cx| {
                                        this.issue_popover_menu = Some((iid, event.position()));
                                        cx.notify();
                                    });
                                }
                            })
                    }),
            )
            .child(meta_row);

        let entity = cx.entity();
        let thread = list(self.issue_thread_list.clone(), move |index, window, app| {
            let item = entity.update(app, |this, cx| {
                this.render_issue_thread_item(index, window, cx)
            });
            // Native `list` lays its rows outside the Styled padding carried by
            // the list element itself. Put the thread gutter on every virtual
            // row so prose and cards never sit against the pane separator.
            div()
                .w_full()
                .px(px(18.0))
                .pt(px(if index == 0 { 16.0 } else { 0.0 }))
                .child(item)
                .into_any_element()
        })
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .bg(ShellDeckColors::bg_surface());

        div()
            .flex_1()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .child(header)
            .child(thread)
            .child(self.render_issue_composer(cx))
            .into_any_element()
    }

    /// The model used by "Proposer une réponse". This belongs in the right
    /// option slot: requests have no alternative delivery destination in the
    /// current API, while the AI backend is a real user setting.
    fn render_support_ai_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use shelldeck_core::ai::AiBackend;
        let model = if self.ai_model.trim().is_empty() {
            self.ai_backend.default_model().to_string()
        } else {
            self.ai_model.trim().to_string()
        };
        let current = self.ai_backend;
        let trigger = div()
            .id("sup-ai-backend")
            .flex()
            .items_center()
            .gap(px(5.0))
            .h(px(26.0))
            .px(px(6.0))
            .rounded(px(7.0))
            .cursor_pointer()
            .text_size(px(11.0))
            .text_color(ShellDeckColors::text_muted())
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .child(ai_provider_inline(current, &model))
            .child(
                svg()
                    .path(lucide_path("chevron-down"))
                    .size(px(11.0))
                    .flex_shrink_0()
                    .text_color(ShellDeckColors::text_muted()),
            );
        let parent = cx.entity();

        Popover::new("sup-ai-backend-popover")
            .anchor(Corner::BottomRight)
            .trigger(trigger)
            .content(move |window, cx| {
                let parent = parent.clone();
                cx.new(move |content_cx| {
                    PopoverContent::new(window, content_cx, move |_window, cx| {
                        let mut list = div().w(px(208.0)).flex().flex_col().gap(px(1.0));
                        for (index, (backend, label)) in [
                            (AiBackend::ClaudeCli, "Claude Code CLI"),
                            (AiBackend::CodexCli, "Codex CLI"),
                            (AiBackend::AiderCli, "Aider CLI"),
                            (AiBackend::OpenAi, "OpenAI API"),
                            (AiBackend::Anthropic, "Anthropic API"),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let selected = backend == current;
                            let row_parent = parent.clone();
                            list = list.child(
                                div()
                                    .id(("sup-ai-opt", index))
                                    .h(px(32.0))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .px(px(8.0))
                                    .rounded(px(7.0))
                                    .cursor_pointer()
                                    .text_size(px(12.0))
                                    .when(selected, |row| row.bg(ShellDeckColors::selected_bg()))
                                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                                    .child(ai_provider_icon(
                                        backend,
                                        14.0,
                                        ShellDeckColors::text_primary(),
                                    ))
                                    .child(div().flex_1().min_w(px(0.0)).child(label))
                                    .when(selected, |row| {
                                        row.child(lucide_icon(
                                            "check",
                                            13.0,
                                            ShellDeckColors::primary(),
                                        ))
                                    })
                                    .on_click(cx.listener(
                                        move |_content, _: &ClickEvent, _, cx| {
                                            row_parent.update(cx, |_this, cx| {
                                                cx.emit(SupportViewEvent::SelectAiBackend(backend));
                                            });
                                            cx.emit(DismissEvent);
                                        },
                                    )),
                            );
                        }
                        list.into_any_element()
                    })
                })
            })
    }

    /// The AI reply card as `.thr-ai-draft` in the mockup — a proposal to
    /// review, not a keystroke. It sits above the composer so `Publier`
    /// prepends into the user's current text (whatever they had is preserved).
    fn render_issue_ai_draft_card(
        &self,
        body: String,
        model: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let issue_id = self.issue_selected.clone().unwrap_or_default();
        let title = if model.trim().is_empty() {
            t!("support.issue.ai_draft").to_string()
        } else {
            t!("support.issue.ai_draft_model", model = model).to_string()
        };
        let leading = vec![
            Button::new(
                "issue-ai-regenerate",
                t!("support.issue.ai_regenerate").to_string(),
            )
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("rotate-ccw"))
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(SupportViewEvent::SuggestIssueReply(issue_id.clone()));
            }))
            .into_any_element(),
            Button::new("issue-ai-edit", t!("support.issue.ai_edit").to_string())
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .icon(IconSource::from("pencil"))
                .on_click(cx.listener(|_, _, _, cx| {
                    cx.emit(SupportViewEvent::PublishIssueAiDraft);
                }))
                .into_any_element(),
        ];
        let trailing = vec![
            Button::new(
                "issue-ai-discard",
                t!("support.issue.ai_discard").to_string(),
            )
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .on_click(cx.listener(|_, _, _, cx| {
                cx.emit(SupportViewEvent::DiscardIssueAiDraft);
            }))
            .into_any_element(),
            Button::new(
                "issue-ai-publish",
                t!("support.issue.ai_publish").to_string(),
            )
            .variant(ButtonVariant::Ai)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("arrow-up"))
            .on_click(cx.listener(|_, _, _, cx| {
                cx.emit(SupportViewEvent::PublishIssueAiDraft);
            }))
            .into_any_element(),
        ];
        ai_draft_card(title, body, leading, trailing)
    }

    pub(super) fn render_issue_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let issue_id = self.issue_selected.clone();
        let placeholder = self
            .issue_detail
            .as_ref()
            .map(|issue| issue.tenant_name.trim())
            .filter(|tenant| !tenant.is_empty())
            .map(|tenant| t!("support.issue.reply_placeholder", tenant = tenant).to_string())
            .unwrap_or_else(|| t!("support.issue_comment_placeholder").to_string());
        div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(2.0))
            .px(px(16.0))
            .pt(px(10.0))
            .pb(px(14.0))
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                if this.paste_attachment(cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .child({
                // The shared Composer owns its multiline Input and therefore
                // one min/max measurement, one viewport and one scroll state.
                let send_entity = cx.entity();
                let ai_issue_id = issue_id.clone().unwrap_or_default();
                let empty = self.composer_state.read(cx).content().trim().is_empty();
                let mut frame = Composer::new("sup-issue-composer", &self.composer_state)
                    .placeholder(placeholder)
                    .min_rows(1)
                    .max_rows(7)
                    // Grey while there is nothing to send, like every other
                    // composer in the app.
                    .commit_enabled(!self.attachment_busy && !empty)
                    .action(
                        IconButton::new("plus")
                            .variant(if self.attachment_panel_open {
                                ButtonVariant::Secondary
                            } else {
                                ButtonVariant::Ghost
                            })
                            .size(gpui::px(28.0))
                            .icon_size(gpui::px(14.0))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.attachment_panel_open = !this.attachment_panel_open;
                                cx.notify();
                            })),
                    )
                    .on_commit(move |cx| {
                        send_entity.update(cx, |this, cx| this.send_composer(cx));
                    })
                    .footnote(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(9.0))
                            .child(t!("ai.assistant.hint.send").to_string())
                            .child(t!("ai.assistant.hint.newline").to_string()),
                    );
                if self.ai_reply_enabled {
                    frame = frame.action(
                        compact_composer_action(
                            "issue-ai-reply",
                            "sparkles",
                            t!("ai.workflow.issue_reply").to_string(),
                            !self.issue_ai_pending,
                        )
                        .when(!self.issue_ai_pending, |action| {
                            action.on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                cx.emit(SupportViewEvent::SuggestIssueReply(ai_issue_id.clone()));
                            }))
                        }),
                    );
                }
                frame = frame.option(self.render_support_ai_picker(cx));
                frame
            })
            .when(self.attachment_panel_open, |composer| {
                composer.child(self.render_attachment_picker(cx))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::thread_scroll_to_restore;

    // SDTEST-1599 — a periodic detail refresh may rebuild measured timeline
    // rows, but it restores an active reading position. A new selection and a
    // reader already pinned to the bottom intentionally keep bottom alignment.
    #[test]
    fn thread_refresh_preserves_reading_position_but_not_new_or_bottom_threads() {
        let reading = gpui::ListOffset {
            item_ix: 4,
            offset_in_item: gpui::px(7.0),
        };
        let restored = thread_scroll_to_restore(true, 13, reading).unwrap();
        assert_eq!(restored.item_ix, 4);
        assert_eq!(restored.offset_in_item, gpui::px(7.0));

        assert!(thread_scroll_to_restore(false, 13, reading).is_none());
        assert!(thread_scroll_to_restore(
            true,
            13,
            gpui::ListOffset {
                item_ix: 13,
                offset_in_item: gpui::px(0.0),
            },
        )
        .is_none());
    }
}
