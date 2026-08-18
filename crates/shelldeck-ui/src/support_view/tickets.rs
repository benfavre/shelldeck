use super::thread::{
    ai_draft_card, attributed_quote, day_separator, delivery_status, human_message,
    human_message_continuation, local_draft, markdown_blocks, message_action, note as thread_note,
    thread_header_picker, thread_picker_option_row, thread_priority_color, thread_status_color,
    timeline_day, timeline_day_label, typing_indicator, HumanMessageMeta, ThreadDeliveryTone,
    ThreadMessageExtras, ThreadNoteKind,
};
use super::*;
use adabraka_ui::prelude::{Composer, ComposerCommit};

#[derive(Clone, Copy)]
enum TicketTimelineGroup {
    Message(usize),
    Typing(usize),
    AiDraft,
    LocalDraft,
}

impl SupportView {
    pub(super) fn rebuild_ticket_thread_cache(&mut self) {
        let Some(ticket) = &self.detail else {
            self.ticket_message_blocks.clear();
            self.ticket_thread_rows.clear();
            self.ticket_thread_list.reset(0);
            return;
        };
        self.ticket_message_blocks = ticket
            .messages
            .iter()
            .map(|message| {
                if message.is_note() {
                    Vec::new()
                } else {
                    markdown_blocks(&message.text)
                }
            })
            .collect();

        if ticket.messages.is_empty() {
            self.ticket_thread_rows = vec![TicketThreadRow::Empty];
            self.ticket_thread_list.reset(1);
            return;
        }

        let mut groups = ticket
            .messages
            .iter()
            .enumerate()
            .map(|(index, message)| (message.at, index, TicketTimelineGroup::Message(index)))
            .collect::<Vec<_>>();
        groups.extend(
            ticket
                .thread_state
                .typing
                .iter()
                .enumerate()
                .map(|(index, typing)| {
                    (
                        typing.at,
                        ticket.messages.len() + index,
                        TicketTimelineGroup::Typing(index),
                    )
                }),
        );
        if let Some(draft) = ticket
            .thread_state
            .suggested_reply
            .as_ref()
            .filter(|draft| !draft.body.trim().is_empty())
        {
            groups.push((draft.at, usize::MAX - 1, TicketTimelineGroup::AiDraft));
        }
        if let Some(draft) = ticket
            .thread_state
            .local_draft
            .as_ref()
            .filter(|draft| !draft.body.trim().is_empty())
        {
            groups.push((draft.at, usize::MAX, TicketTimelineGroup::LocalDraft));
        }
        groups.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

        let mut rows = Vec::new();
        let mut previous_day = None;
        for (at, _, group) in groups {
            let day = timeline_day(at);
            if previous_day.is_some() && day.is_some() && day != previous_day {
                rows.push(TicketThreadRow::Day { at });
            }
            if day.is_some() {
                previous_day = day;
            }
            match group {
                TicketTimelineGroup::Message(message) => {
                    let count = if ticket.messages[message].is_note() {
                        1
                    } else {
                        self.ticket_message_blocks[message].len().max(1)
                    };
                    rows.extend((0..count).map(|block| TicketThreadRow::Message {
                        message,
                        block,
                        first: block == 0,
                        last: block + 1 == count,
                    }));
                }
                TicketTimelineGroup::Typing(index) => rows.push(TicketThreadRow::Typing { index }),
                TicketTimelineGroup::AiDraft => rows.push(TicketThreadRow::AiDraft),
                TicketTimelineGroup::LocalDraft => rows.push(TicketThreadRow::LocalDraft),
            }
        }
        self.ticket_thread_rows = rows;
        self.ticket_thread_list.reset(self.ticket_thread_rows.len());
    }

    fn ticket_thread_item_count(&self) -> usize {
        self.ticket_thread_rows.len()
    }

    pub(super) fn render_jean_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
    pub(super) fn compact_filter_button(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
    ) -> Button {
        Button::new(id, label)
            .size(ButtonSize::Sm)
            .h(gpui::px(26.0))
            .px(gpui::px(8.0))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_pick_button(
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

    pub(super) fn render_modal_pick_row(
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

    pub(super) fn render_applied_filter_chip(
        &self,
        id: String,
        icon: &str,
        label: String,
        cx: &mut Context<Self>,
        on_clear: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        self.render_applied_filter_chip_with_badge(
            id,
            icon,
            Badge::new(label).variant(BadgeVariant::Outline),
            cx,
            on_clear,
        )
    }

    /// Same shape as `render_applied_filter_chip` but the caller supplies a
    /// pre-built `Badge` so we can inject the colored `priority_badge` (or
    /// `issue_status_badge`, …) instead of the default Outline label. Kept
    /// private to preserve the harmonized icon + gap + IconButton geometry.
    pub(super) fn render_applied_filter_chip_with_badge(
        &self,
        id: String,
        icon: &str,
        badge: Badge,
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
                    .child(badge),
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

    pub(super) fn render_applied_filter_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
    pub(super) fn render_filter_modal(&self, cx: &mut Context<Self>) -> impl IntoElement {
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

    pub(super) fn render_filters(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                div().flex_1().child(
                    Input::new(&self.search_state)
                        .size(InputSize::Sm)
                        .placeholder(t!("support.search_placeholder").to_string())
                        .prefix(lucide_icon("search", 12.0, ShellDeckColors::text_muted()))
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
                            Badge::new(active_adv_count.to_string()).variant(BadgeVariant::Default),
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
                    .child(Badge::new(count.to_string()).variant(BadgeVariant::Secondary)),
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
    pub(super) fn render_ticket_row(
        &self,
        t: &SupportTicket,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id_click = t.id.clone();
        let id_rclick = t.id.clone();
        let id_kebab = t.id.clone();
        let selected = self.selected_id.as_deref() == Some(t.id.as_str());
        let subject = if t.subject.trim().is_empty() {
            "(sans objet)".to_string()
        } else {
            crate::external_content::external_title(&t.subject)
        };
        let group_name = SharedString::from(format!("tk-row-{}", t.id));

        let mut row = div()
            .id(ElementId::from(SharedString::from(format!("tk-{}", t.id))))
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
            .id(ElementId::from(SharedString::from(format!(
                "tk-kebab-{}",
                t.id
            ))))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .w(px(22.0))
            .h(px(22.0))
            .rounded(px(4.0))
            .text_color(ShellDeckColors::text_muted())
            .opacity(0.0)
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
            .child(lucide_icon(
                "ellipsis-vertical",
                14.0,
                ShellDeckColors::text_muted(),
            ));

        // Match Requests exactly: the subject owns the first line; compact
        // state and source metadata live below it. Ticket fields remain
        // independent and are only adapted to that shared presentation.
        let subject_weight = if selected || t.unread {
            FontWeight::SEMIBOLD
        } else {
            FontWeight::MEDIUM
        };
        let channel = if t.channel.trim().is_empty() {
            "—".to_string()
        } else {
            t.channel.clone()
        };
        let mut meta = format!("{} · {}", t.contact.display(), channel);
        if t.msg_count > 0 {
            meta.push_str(&format!(" · {}", t.msg_count));
        }
        let when = rel_time(t.last_at);
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
                            .font_weight(subject_weight)
                            .text_color(ShellDeckColors::text_primary())
                            .child(subject),
                    )
                    .child(kebab),
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
                            .child(
                                div()
                                    .w(px(6.0))
                                    .h(px(6.0))
                                    .rounded_full()
                                    .bg(thread_status_color(&t.status)),
                            )
                            .child(status_label(&t.status)),
                    )
                    .when(
                        t.priority != "normal" && !t.priority.trim().is_empty(),
                        |el| el.child(div().flex_shrink_0().child(priority_badge(&t.priority))),
                    )
                    .child(div().flex_1().min_w(px(0.0)).truncate().child(meta))
                    .when(!when.is_empty(), |el| {
                        el.child(div().flex_shrink_0().child(when.clone()))
                    }),
            );
        row
    }

    fn render_message_segment(
        &self,
        msg: &SupportMessage,
        body: SharedString,
        first: bool,
        last: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                    let name = self.me.name.trim();
                    if !name.is_empty() {
                        Some(name.to_string())
                    } else {
                        let email = self.me.email.trim();
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
            .unwrap_or_else(|| {
                if msg.is_customer() {
                    t!("support.bubble.client").to_string()
                } else {
                    t!("support.bubble.agent").to_string()
                }
            });

        let attachments = (last && !msg.attachments.is_empty())
            .then(|| self.render_issue_attachment_links(&msg.attachments, cx));
        let channel = if msg.channel.trim().is_empty() {
            self.detail
                .as_ref()
                .map(|ticket| ticket.channel.as_str())
                .unwrap_or_default()
        } else {
            msg.channel.as_str()
        };
        let extras = ThreadMessageExtras {
            quote: first
                .then(|| {
                    msg.quote
                        .as_ref()
                        .map(|quote| attributed_quote(quote.author.clone(), quote.body.clone()))
                })
                .flatten(),
            delivery: last.then(|| self.render_ticket_delivery(msg)).flatten(),
            actions: first.then(|| self.render_ticket_message_actions(msg, cx)),
            group: Some(SharedString::from(format!(
                "ticket-message-{}",
                msg.at.to_bits()
            ))),
            link_handler: Some(Self::thread_link_handler(cx)),
        };
        let font_size = px(12.5).to_pixels(window.rem_size());
        if first {
            human_message(
                HumanMessageMeta {
                    author: who.into(),
                    mine: !msg.is_customer(),
                    at: msg.at,
                    channel: (!channel.trim().is_empty())
                        .then(|| SharedString::from(channel.to_string())),
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

    fn render_ticket_message_actions(
        &self,
        message: &SupportMessage,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity();
        let reply_entity = entity.clone();
        let focus = self.composer_state.read(cx).focus_handle(cx);
        let author = message
            .name
            .clone()
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| {
                if message.is_customer() {
                    t!("support.bubble.client").to_string()
                } else {
                    t!("support.bubble.agent").to_string()
                }
            });
        let quoted = message.text.clone();
        let ticket_id = self.selected_id.clone().unwrap_or_default();
        div()
            .flex()
            .items_center()
            .gap(px(4.0))
            .child(message_action(
                SharedString::from(format!("ticket-reply-{}", message.at.to_bits())),
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
                SharedString::from(format!("ticket-copy-{}", message.at.to_bits())),
                "copy",
                t!("support.thread.copy").to_string(),
                {
                    let body = message.text.clone();
                    move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(body.clone()));
                    }
                },
            ))
            .when(self.ai_reply_enabled, |actions| {
                actions.child(message_action(
                    SharedString::from(format!("ticket-ai-{}", message.at.to_bits())),
                    "sparkles",
                    t!("support.thread.rewrite_ai").to_string(),
                    move |_, _, cx| {
                        entity.update(cx, |_, cx| {
                            cx.emit(SupportViewEvent::SuggestReply(ticket_id.clone()));
                        });
                    },
                ))
            })
            .into_any_element()
    }

    fn render_ticket_delivery(&self, message: &SupportMessage) -> Option<AnyElement> {
        let delivery = message.delivery.as_ref()?;
        let channel = if delivery.channel.trim().is_empty() {
            if message.channel.trim().is_empty() {
                self.detail
                    .as_ref()
                    .map(|ticket| ticket.channel.as_str())
                    .unwrap_or("support")
            } else {
                message.channel.as_str()
            }
        } else {
            delivery.channel.as_str()
        };
        if delivery.status == "failed" {
            // Future API hook: Support has no idempotent message retry route
            // yet. Keep the affordance in the fixture without issuing a fake
            // write; the event can be wired when Manage exposes message ids.
            let retry = message_action(
                SharedString::from(format!("ticket-retry-{}", message.at.to_bits())),
                "rotate-ccw",
                t!("support.thread.retry").to_string(),
                |_, _, _| {},
            );
            Some(delivery_status(
                if delivery.error.trim().is_empty() {
                    t!("support.thread.send_failed").to_string()
                } else {
                    delivery.error.clone()
                },
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

    fn ticket_note_kind(message: &SupportMessage) -> ThreadNoteKind {
        match message.kind.as_str() {
            "status" => ThreadNoteKind::Status,
            "github" => ThreadNoteKind::Github,
            "dispatch" => ThreadNoteKind::Dispatch,
            "system" => ThreadNoteKind::System,
            _ => ThreadNoteKind::Internal,
        }
    }

    fn apply_ticket_ai_draft(&mut self, cx: &mut Context<Self>) {
        let Some(body) = self
            .detail
            .as_ref()
            .and_then(|ticket| ticket.thread_state.suggested_reply.as_ref())
            .map(|draft| draft.body.clone())
        else {
            return;
        };
        self.composer_state
            .update(cx, |state, cx| state.replace_content(body, cx));
        cx.notify();
    }

    fn discard_ticket_ai_draft(&mut self, cx: &mut Context<Self>) {
        if let Some(ticket) = self.detail.as_mut() {
            ticket.thread_state.suggested_reply = None;
        }
        self.rebuild_ticket_thread_cache();
        cx.notify();
    }

    fn render_ticket_ai_draft_card(
        &self,
        body: String,
        model: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let title = if model.trim().is_empty() {
            t!("support.issue.ai_draft").to_string()
        } else {
            t!("support.issue.ai_draft_model", model = model).to_string()
        };
        let ticket_id = self.selected_id.clone().unwrap_or_default();
        let leading = vec![
            Button::new(
                "ticket-ai-regenerate",
                t!("support.issue.ai_regenerate").to_string(),
            )
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("rotate-ccw"))
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(SupportViewEvent::SuggestReply(ticket_id.clone()));
            }))
            .into_any_element(),
            Button::new("ticket-ai-edit", t!("support.issue.ai_edit").to_string())
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::Sm)
                .icon(IconSource::from("pencil"))
                .on_click(cx.listener(|this, _, _, cx| this.apply_ticket_ai_draft(cx)))
                .into_any_element(),
        ];
        let trailing = vec![
            Button::new(
                "ticket-ai-discard",
                t!("support.issue.ai_discard").to_string(),
            )
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .on_click(cx.listener(|this, _, _, cx| this.discard_ticket_ai_draft(cx)))
            .into_any_element(),
            Button::new(
                "ticket-ai-publish",
                t!("support.issue.ai_publish").to_string(),
            )
            .variant(ButtonVariant::Ai)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("arrow-up"))
            .on_click(cx.listener(|this, _, _, cx| this.apply_ticket_ai_draft(cx)))
            .into_any_element(),
        ];
        ai_draft_card(title, body, leading, trailing)
    }

    fn render_ticket_thread_item(
        &self,
        index: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let final_item = index + 1 == self.ticket_thread_item_count();
        let object_bottom = if final_item { 18.0 } else { 20.0 };
        let Some(ticket) = &self.detail else {
            return div().into_any_element();
        };
        let Some(row) = self.ticket_thread_rows.get(index).cloned() else {
            return div().into_any_element();
        };
        let (content, bottom) = match row {
            TicketThreadRow::Empty => (
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.empty.messages").to_string())
                    .into_any_element(),
                object_bottom,
            ),
            TicketThreadRow::Message {
                message: message_index,
                block,
                first,
                last,
            } => {
                let message = &ticket.messages[message_index];
                if message.is_note() {
                    (
                        thread_note(
                            message.text.clone(),
                            message.name.clone(),
                            message.at,
                            Self::ticket_note_kind(message),
                            px(11.5).to_pixels(window.rem_size()),
                        ),
                        object_bottom,
                    )
                } else {
                    let body = self.ticket_message_blocks[message_index]
                        .get(block)
                        .cloned()
                        .unwrap_or_else(|| SharedString::from(message.text.clone()));
                    (
                        self.render_message_segment(message, body, first, last, window, cx),
                        if last { object_bottom } else { 8.0 },
                    )
                }
            }
            TicketThreadRow::Day { at } => (day_separator(timeline_day_label(at)), 14.0),
            TicketThreadRow::Typing { index } => (
                typing_indicator(
                    ticket
                        .thread_state
                        .typing
                        .get(index)
                        .map(|typing| typing.author.clone())
                        .unwrap_or_default(),
                ),
                object_bottom,
            ),
            TicketThreadRow::AiDraft => {
                let Some(draft) = ticket.thread_state.suggested_reply.as_ref() else {
                    return div().into_any_element();
                };
                (
                    self.render_ticket_ai_draft_card(draft.body.clone(), draft.model.clone(), cx),
                    object_bottom,
                )
            }
            TicketThreadRow::LocalDraft => (
                local_draft(
                    ticket
                        .thread_state
                        .local_draft
                        .as_ref()
                        .map(|draft| draft.body.clone())
                        .unwrap_or_default(),
                ),
                object_bottom,
            ),
        };
        div()
            .w_full()
            .pb(px(bottom))
            .child(content)
            .into_any_element()
    }

    pub(super) fn close_popover_menu(&mut self, cx: &mut Context<Self>) {
        self.popover_menu = None;
        cx.notify();
    }

    pub(super) fn ticket_for_menu<'a>(
        &'a self,
        kind: &SupportMenuKind,
    ) -> Option<&'a SupportTicket> {
        match kind {
            SupportMenuKind::ConversationHeader => self.detail.as_ref(),
            SupportMenuKind::TicketList(id) => self.tickets.iter().find(|t| &t.id == id),
        }
    }

    pub(super) fn jean_text_for_ticket(t: &SupportTicket) -> String {
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

    pub(super) fn build_ticket_menu_items(
        &self,
        kind: &SupportMenuKind,
        entity: Entity<SupportView>,
    ) -> Vec<PopoverMenuItem> {
        let Some(ticket) = self.ticket_for_menu(kind) else {
            return vec![];
        };
        let id = ticket.id.clone();
        let is_pending = ticket.status == "pending";
        let is_mine =
            !self.my_email().is_empty() && ticket.assignee.eq_ignore_ascii_case(self.my_email());
        let (status_next, menu_status_label) = if is_pending {
            ("open".to_string(), t!("support.menu.reopen").to_string())
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
                    .icon(if is_pending { "circle-check" } else { "clock" })
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
            // Status, priority and assignee are direct header controls now.
            // Keep the kebab for secondary intentions only, matching Requests.
            if self.ai_reply_enabled {
                let triage_id = id.clone();
                items.push(
                    PopoverMenuItem::new(
                        "menu-triage-ai",
                        t!("ai.workflow.support_triage").to_string(),
                    )
                    .icon("sparkles")
                    .on_click({
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_popover_menu(cx);
                                cx.emit(SupportViewEvent::TriageTicket(triage_id.clone()));
                            });
                        }
                    }),
                );
            }
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

    pub(super) fn render_ticket_popover(
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
    pub(super) fn render_empty_conversation(&self) -> Div {
        support_empty_detail(
            div()
                .text_size(px(22.0))
                .text_color(ShellDeckColors::primary())
                .child("💬"),
            t!("support.empty.tickets").to_string(),
            t!("support.empty.tickets_hint").to_string(),
        )
    }

    pub(crate) fn update_ticket_showcase(
        &mut self,
        id: &str,
        update: impl Fn(&mut SupportTicket),
        cx: &mut Context<Self>,
    ) -> bool {
        if id != SUPPORT_TICKET_SHOWCASE_ID {
            return false;
        }
        let Some(old) = self.tickets.iter().find(|ticket| ticket.id == id).cloned() else {
            cx.notify();
            return true;
        };
        let mut updated = self
            .detail
            .as_ref()
            .filter(|ticket| ticket.id == id)
            .cloned()
            .unwrap_or_else(|| old.clone());
        update(&mut updated);

        if old.status != updated.status {
            match old.status.as_str() {
                "open" => self.counts.open = self.counts.open.saturating_sub(1),
                "pending" => self.counts.pending = self.counts.pending.saturating_sub(1),
                "closed" => self.counts.closed = self.counts.closed.saturating_sub(1),
                _ => {}
            }
            match updated.status.as_str() {
                "open" => self.counts.open = self.counts.open.saturating_add(1),
                "pending" => self.counts.pending = self.counts.pending.saturating_add(1),
                "closed" => self.counts.closed = self.counts.closed.saturating_add(1),
                _ => {}
            }
        }

        if old.assignee != updated.assignee {
            let my_email = self.me.email.trim();
            let old_unassigned = old.assignee.trim().is_empty();
            let new_unassigned = updated.assignee.trim().is_empty();
            let old_mine = !my_email.is_empty() && old.assignee.eq_ignore_ascii_case(my_email);
            let new_mine = !my_email.is_empty() && updated.assignee.eq_ignore_ascii_case(my_email);
            if old_unassigned != new_unassigned {
                self.counts.unassigned = if new_unassigned {
                    self.counts.unassigned.saturating_add(1)
                } else {
                    self.counts.unassigned.saturating_sub(1)
                };
            }
            if old_mine != new_mine {
                self.counts.mine = if new_mine {
                    self.counts.mine.saturating_add(1)
                } else {
                    self.counts.mine.saturating_sub(1)
                };
            }
        }

        if let Some(row) = self.tickets.iter_mut().find(|row| row.id == id) {
            *row = updated.clone();
        }
        if self.detail.as_ref().is_some_and(|ticket| ticket.id == id) {
            self.detail = Some(updated);
        }
        cx.notify();
        true
    }

    /// Keep the staff-only fixture fully interactive without ever sending its
    /// synthetic id or attachments to Manage.
    pub(crate) fn append_ticket_showcase_message(
        &mut self,
        id: &str,
        text: String,
        note: bool,
        attachments: Vec<AttachmentDraft>,
        cx: &mut Context<Self>,
    ) -> bool {
        if id != SUPPORT_TICKET_SHOWCASE_ID {
            return false;
        }
        let Some(mut ticket) = self.detail.clone().filter(|ticket| ticket.id == id) else {
            return false;
        };
        let now = chrono::Utc::now().timestamp_millis() as f64;
        let sender = if self.me.name.trim().is_empty() {
            self.me.email.clone()
        } else {
            self.me.name.clone()
        };
        let attachments = attachments
            .into_iter()
            .enumerate()
            .map(
                |(index, draft)| shelldeck_core::config::issues::IssueAttachment {
                    id: format!("fake-ticket-upload-{now:.0}-{index}"),
                    filename: draft.filename,
                    content_type: draft.content_type,
                    bytes: draft.bytes.len() as u64,
                    created_by: sender.clone(),
                    created_at: now,
                    ..Default::default()
                },
            )
            .collect();
        ticket.messages.push(SupportMessage {
            from: if note { "note" } else { "agent" }.to_string(),
            text: text.clone(),
            at: now,
            name: Some(sender),
            attachments,
            kind: if note { "internal" } else { "comment" }.to_string(),
            channel: ticket.channel.clone(),
            delivery: (!note).then_some(SupportMessageDelivery {
                status: "sent".to_string(),
                channel: ticket.channel.clone(),
                at: now,
                error: String::new(),
            }),
            ..Default::default()
        });
        ticket.last_at = now;
        ticket.last_preview = text;
        ticket.msg_count = ticket.msg_count.saturating_add(1);
        self.set_detail(ticket, cx);
        true
    }

    fn render_ticket_status_picker(
        &self,
        ticket: &SupportTicket,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let trigger = thread_header_picker(
            "ticket-detail-status",
            div()
                .size(px(6.0))
                .rounded_full()
                .bg(thread_status_color(&ticket.status)),
            status_label(&ticket.status),
            self.me.staff,
        );
        if !self.me.staff {
            return trigger;
        }
        let parent = cx.entity();
        let ticket_id = ticket.id.clone();
        let current = ticket.status.clone();
        Popover::new("ticket-detail-status-popover")
            .trigger(trigger)
            .content(move |window, cx| {
                let parent = parent.clone();
                let ticket_id = ticket_id.clone();
                let current = current.clone();
                cx.new(move |content_cx| {
                    PopoverContent::new(window, content_cx, move |_window, cx| {
                        let mut list = div().w(px(176.0)).flex().flex_col().gap(px(2.0));
                        for status in ["open", "pending", "closed"] {
                            let row_parent = parent.clone();
                            let row_ticket_id = ticket_id.clone();
                            list = list.child(
                                thread_picker_option_row(
                                    format!("ticket-status-option-{status}").into(),
                                    div()
                                        .size(px(7.0))
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .bg(thread_status_color(status)),
                                    status_label(status),
                                    None,
                                    current == status,
                                )
                                .on_click(cx.listener(
                                    move |_content, _: &ClickEvent, _, cx| {
                                        row_parent.update(cx, |this, cx| {
                                            if !this.update_ticket_showcase(
                                                &row_ticket_id,
                                                |ticket| ticket.status = status.to_string(),
                                                cx,
                                            ) {
                                                cx.emit(SupportViewEvent::SetStatus {
                                                    id: row_ticket_id.clone(),
                                                    status: status.to_string(),
                                                });
                                            }
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

    fn render_ticket_priority_picker(
        &self,
        ticket: &SupportTicket,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let trigger = thread_header_picker(
            "ticket-detail-priority",
            div()
                .size(px(6.0))
                .rounded_full()
                .bg(thread_priority_color(&ticket.priority)),
            priority_label(&ticket.priority),
            self.me.staff,
        );
        if !self.me.staff {
            return trigger;
        }
        let parent = cx.entity();
        let ticket_id = ticket.id.clone();
        let current = ticket.priority.clone();
        Popover::new("ticket-detail-priority-popover")
            .trigger(trigger)
            .content(move |window, cx| {
                let parent = parent.clone();
                let ticket_id = ticket_id.clone();
                let current = current.clone();
                cx.new(move |content_cx| {
                    PopoverContent::new(window, content_cx, move |_window, cx| {
                        let mut list = div().w(px(176.0)).flex().flex_col().gap(px(2.0));
                        for priority in ["low", "normal", "high", "urgent"] {
                            let row_parent = parent.clone();
                            let row_ticket_id = ticket_id.clone();
                            list = list.child(
                                thread_picker_option_row(
                                    format!("ticket-priority-option-{priority}").into(),
                                    div()
                                        .size(px(7.0))
                                        .flex_shrink_0()
                                        .rounded_full()
                                        .bg(thread_priority_color(priority)),
                                    priority_label(priority),
                                    None,
                                    current == priority,
                                )
                                .on_click(cx.listener(
                                    move |_content, _: &ClickEvent, _, cx| {
                                        row_parent.update(cx, |this, cx| {
                                            if !this.update_ticket_showcase(
                                                &row_ticket_id,
                                                |ticket| ticket.priority = priority.to_string(),
                                                cx,
                                            ) {
                                                cx.emit(SupportViewEvent::SetPriority {
                                                    id: row_ticket_id.clone(),
                                                    priority: priority.to_string(),
                                                });
                                            }
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

    fn render_ticket_assignee_picker(
        &self,
        ticket: &SupportTicket,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let trigger = thread_header_picker(
            "ticket-detail-assignee",
            lucide_icon("at-sign", 11.0, ShellDeckColors::text_muted()),
            self.assignee_label(&ticket.assignee),
            self.me.staff,
        );
        if !self.me.staff {
            return trigger;
        }

        let mut agents = self
            .agents
            .iter()
            .filter(|agent| !agent.email.trim().is_empty())
            .cloned()
            .collect::<Vec<_>>();
        agents.sort_by_key(|agent| {
            if agent.name.trim().is_empty() {
                agent.email.to_lowercase()
            } else {
                agent.name.to_lowercase()
            }
        });
        agents.dedup_by(|a, b| a.email.eq_ignore_ascii_case(&b.email));

        let total = agents.len();
        let parent = cx.entity();
        let ticket_id = ticket.id.clone();
        let current = ticket.assignee.clone();
        let me_name = self.me.name.clone();
        let me_email = self.me.email.clone();
        let search = self.issue_assignee_search_state.clone();
        Popover::new("ticket-detail-assignee-popover")
            .trigger(trigger)
            .content(move |window, cx| {
                search.update(cx, InputState::reset);
                let parent = parent.clone();
                let ticket_id = ticket_id.clone();
                let current = current.clone();
                let me_name = me_name.clone();
                let me_email = me_email.clone();
                let search = search.clone();
                let agents = agents.clone();
                cx.new(move |content_cx| {
                    PopoverContent::new(window, content_cx, move |_window, cx| {
                        let query = search.read(cx).content().trim().to_lowercase();
                        let filtered = agents
                            .iter()
                            .filter(|agent| {
                                query.is_empty()
                                    || agent.name.to_lowercase().contains(&query)
                                    || agent.email.to_lowercase().contains(&query)
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        let filtered_count = filtered.len();
                        let list_height = px((filtered_count.clamp(1, 5) as f32) * 40.0);
                        let filtered = Rc::new(filtered);
                        let content_entity = cx.entity();
                        let rows_parent = parent.clone();
                        let rows_ticket_id = ticket_id.clone();
                        let rows_current = current.clone();
                        let rows = filtered.clone();
                        let none_parent = parent.clone();
                        let none_ticket_id = ticket_id.clone();
                        let me_parent = parent.clone();
                        let me_ticket_id = ticket_id.clone();
                        let me_label = if me_name.trim().is_empty() {
                            t!("support.assignee.me").to_string()
                        } else {
                            format!("{} · {me_name}", t!("support.assignee.me"))
                        };
                        let me_click_email = me_email.clone();

                        div()
                            .w(px(320.0))
                            .flex()
                            .flex_col()
                            .gap(px(5.0))
                            .child(
                                Input::new(&search)
                                    .size(InputSize::Sm)
                                    .placeholder(
                                        t!("support.issues.assignee.picker.search").to_string(),
                                    )
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
                                            "ticket-assignee-none".into(),
                                            lucide_icon(
                                                "at-sign",
                                                12.0,
                                                ShellDeckColors::text_muted(),
                                            ),
                                            t!("support.assignee.none").to_string(),
                                            None,
                                            current.trim().is_empty(),
                                        )
                                        .on_click(cx.listener(
                                            move |_content, _: &ClickEvent, _, cx| {
                                                none_parent.update(cx, |this, cx| {
                                                    if !this.update_ticket_showcase(
                                                        &none_ticket_id,
                                                        |ticket| ticket.assignee.clear(),
                                                        cx,
                                                    ) {
                                                        cx.emit(SupportViewEvent::Assign {
                                                            id: none_ticket_id.clone(),
                                                            assignee: String::new(),
                                                        });
                                                    }
                                                });
                                                cx.emit(DismissEvent);
                                            },
                                        )),
                                    )
                                    .child(
                                        thread_picker_option_row(
                                            "ticket-assignee-me".into(),
                                            lucide_icon(
                                                "at-sign",
                                                12.0,
                                                ShellDeckColors::text_muted(),
                                            ),
                                            me_label,
                                            None,
                                            current.eq_ignore_ascii_case("me")
                                                || (!me_email.trim().is_empty()
                                                    && current.eq_ignore_ascii_case(&me_email)),
                                        )
                                        .on_click(cx.listener(
                                            move |_content, _: &ClickEvent, _, cx| {
                                                let value = me_click_email.clone();
                                                me_parent.update(cx, |this, cx| {
                                                    if !this.update_ticket_showcase(
                                                        &me_ticket_id,
                                                        |ticket| ticket.assignee = value.clone(),
                                                        cx,
                                                    ) {
                                                        cx.emit(SupportViewEvent::Assign {
                                                            id: me_ticket_id.clone(),
                                                            assignee: "me".to_string(),
                                                        });
                                                    }
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
                                            .child(
                                                t!("support.issues.assignee.no_match").to_string(),
                                            )
                                            .into_any_element()
                                    } else {
                                        uniform_list(
                                            "ticket-header-assignee-options",
                                            filtered_count,
                                            cx.processor(
                                                move |_content,
                                                      range: Range<usize>,
                                                      _window,
                                                      cx| {
                                                    range
                                                        .filter_map(|index| {
                                                            rows
                                                                .get(index)
                                                                .cloned()
                                                                .map(|agent| (index, agent))
                                                        })
                                                        .map(|(index, agent)| {
                                                            let label = if agent.name.trim().is_empty() {
                                                                agent.email.clone()
                                                            } else {
                                                                agent.name.clone()
                                                            };
                                                            let active = rows_current
                                                                .eq_ignore_ascii_case(&agent.email)
                                                                || rows_current.eq_ignore_ascii_case(&label);
                                                            let row_parent = rows_parent.clone();
                                                            let row_ticket_id = rows_ticket_id.clone();
                                                            let value = agent.email.clone();
                                                            thread_picker_option_row(
                                                                format!("ticket-assignee-agent-{index}").into(),
                                                                lucide_icon(
                                                                    "at-sign",
                                                                    12.0,
                                                                    ShellDeckColors::text_muted(),
                                                                ),
                                                                label,
                                                                Some(agent.email.into()),
                                                                active,
                                                            )
                                                            .h(px(40.0))
                                                            .on_click(cx.listener(
                                                                move |_content, _: &ClickEvent, _, cx| {
                                                                    row_parent.update(cx, |this, cx| {
                                                                        if !this.update_ticket_showcase(
                                                                            &row_ticket_id,
                                                                            |ticket| {
                                                                                ticket.assignee = value.clone()
                                                                            },
                                                                            cx,
                                                                        ) {
                                                                            cx.emit(SupportViewEvent::Assign {
                                                                                id: row_ticket_id.clone(),
                                                                                assignee: value.clone(),
                                                                            });
                                                                        }
                                                                    });
                                                                    cx.emit(DismissEvent);
                                                                },
                                                            ))
                                                            .into_any_element()
                                                        })
                                                        .collect::<Vec<_>>()
                                                },
                                            ),
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
                                    .child(
                                        t!("support.issue.assignee_count", count = total)
                                            .to_string(),
                                    ),
                            )
                            .into_any_element()
                    })
                })
            })
            .into_any_element()
    }

    pub(super) fn render_conversation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ticket) = self.detail.clone() else {
            return self.render_empty_conversation();
        };
        let tid = ticket.id.clone();

        // Same presentation contract as Requests: source-specific values feed
        // the shared title/actions row and the same three metadata pickers.
        // Contact + channel replace tenant + site because SupportTicket's wire
        // schema carries different context.
        let contact_name = ticket.contact.display();
        let last_at = ticket.last_at;
        let subject = if ticket.subject.trim().is_empty() {
            t!("support.empty.no_subject").to_string()
        } else {
            crate::external_content::external_title(&ticket.subject)
        };

        let mut context = vec![contact_name];
        if !ticket.channel.trim().is_empty() {
            context.push(ticket.channel.clone());
        }
        let mut context_label = context.join(" · ");
        if last_at > 0.0 {
            context_label.push(' ');
            context_label.push_str(t!("support.last_exchange", time = rel_time(last_at)).as_ref());
        }

        let mut meta_row = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(6.0))
            .child(self.render_ticket_status_picker(&ticket, cx))
            .child(self.render_ticket_priority_picker(&ticket, cx))
            .child(self.render_ticket_assignee_picker(&ticket, cx))
            .child(
                div()
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(context_label),
            );
        for tag in ticket
            .tags
            .iter()
            .filter(|tag| !tag.trim().is_empty())
            .take(2)
        {
            meta_row = meta_row.child(Badge::new(tag.clone()).variant(BadgeVariant::Outline));
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
                            .child(subject),
                    )
                    .child({
                        let entity = cx.entity();
                        let summary_id = tid.clone();
                        let mut actions = div().flex().items_center().flex_shrink_0().gap(px(6.0));
                        if self.ai_reply_enabled {
                            actions = actions.child(
                                Button::new(
                                    "support-ai-summary",
                                    t!("support.issue.summarize").to_string(),
                                )
                                .variant(ButtonVariant::Ghost)
                                .size(ButtonSize::Sm)
                                .h(px(28.0))
                                .px(px(8.0))
                                .icon(IconSource::from("sparkles"))
                                .on_click(cx.listener(
                                    move |_, _, _, cx| {
                                        cx.emit(SupportViewEvent::SummarizeTicket(
                                            summary_id.clone(),
                                        ));
                                    },
                                )),
                            );
                        }
                        actions.child(
                            IconButton::new("ellipsis")
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
                                }),
                        )
                    }),
            )
            .child(meta_row);

        // Variable-height native list: only visible Markdown blocks are
        // parsed, laid out and painted while bottom alignment keeps the chat
        // on its latest exchange when opened.
        let entity = cx.entity();
        let messages = list(
            self.ticket_thread_list.clone(),
            move |index, window, app| {
                let item = entity.update(app, |this, cx| {
                    this.render_ticket_thread_item(index, window, cx)
                });
                // GPUI's native list does not inset rows with padding styled
                // on the list itself. Each virtual row owns the 18 px thread
                // gutter; only the first row owns the 16 px leading inset.
                div()
                    .w_full()
                    .px(px(18.0))
                    .pt(px(if index == 0 { 16.0 } else { 0.0 }))
                    .child(item)
                    .into_any_element()
            },
        )
        .flex_1()
        .min_h(px(0.0))
        .w_full()
        .bg(ShellDeckColors::bg_surface());

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
            .overflow_hidden()
            .child(header)
            .child(messages)
            .child(self.render_composer(&tid, cx))
    }

    pub(super) fn render_attachment_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity().downgrade();
        let previews = render_attachment_draft_gallery(
            &self.attachment_drafts,
            "support-attachment-draft",
            move |index, cx| {
                if let Some(entity) = entity.upgrade() {
                    entity.update(cx, |this, cx| {
                        if index < this.attachment_drafts.len() {
                            this.attachment_drafts.remove(index);
                        }
                        cx.notify();
                    });
                }
            },
        );

        let url_input = Input::new(&self.attachment_url_state)
            .size(InputSize::Sm)
            .placeholder(t!("user.requests.attachments.url_placeholder").to_string())
            .on_enter({
                let entity = cx.entity();
                move |_value, cx| entity.update(cx, |this, cx| this.import_attachment_url(cx))
            });

        div()
            .id("support-attachment-picker")
            .flex()
            .flex_col()
            .gap(px(8.0))
            .pt(px(9.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let mods = event.keystroke.modifiers;
                if event.keystroke.key.eq_ignore_ascii_case("v")
                    && (mods.control || mods.platform)
                    && this.paste_attachment(cx)
                {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                if this.paste_attachment(cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                let generation = this.attachment_generation;
                this.import_attachment_paths(paths.paths().to_vec(), generation, cx);
            }))
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
                            .gap(px(6.0))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(t!("user.requests.attachments.title").to_string()),
                            )
                            .child(
                                Badge::new(format!(
                                    "{}/{}",
                                    self.attachment_drafts.len(),
                                    ISSUE_ATTACHMENT_MAX_COUNT
                                ))
                                .variant(BadgeVariant::Secondary),
                            ),
                    )
                    .child(
                        div()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("user.requests.attachments.drop_hint").to_string()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .flex_wrap()
                    .gap(px(5.0))
                    .child(
                        Button::new(
                            "support-attachment-file",
                            t!("user.requests.attachments.file").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .icon(IconSource::from("upload"))
                        .disabled(self.attachment_busy)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.pick_attachments(window, cx)),
                        ),
                    )
                    .child(
                        Button::new(
                            "support-attachment-paste",
                            t!("user.requests.attachments.paste").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .icon(IconSource::from("clipboard-paste"))
                        .disabled(self.attachment_busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if !this.paste_attachment(cx) {
                                this.error = Some(t!("toast.issue.clipboard_no_image").to_string());
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        Button::new(
                            "support-attachment-capture",
                            t!("user.requests.attachments.capture").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .icon(IconSource::from("scan"))
                        .disabled(self.attachment_busy)
                        .on_click(cx.listener(|this, _, _, cx| this.capture_attachment(cx))),
                    ),
            )
            .when(!self.attachment_drafts.is_empty(), |el| el.child(previews))
            .when(!self.attachment_url_open, |el| {
                el.child(
                    Button::new(
                        "support-attachment-url-toggle",
                        t!("user.requests.attachments.url_toggle").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .icon(IconSource::from("globe"))
                    .disabled(self.attachment_busy)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.attachment_url_open = true;
                        cx.notify();
                    })),
                )
            })
            .when(self.attachment_url_open, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .child(div().flex_1().min_w(px(0.0)).child(url_input))
                        .child(
                            Button::new(
                                "support-attachment-url",
                                t!("user.requests.attachments.add_url").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Outline)
                            .icon(IconSource::from("globe"))
                            .disabled(self.attachment_busy)
                            .on_click(cx.listener(|this, _, _, cx| this.import_attachment_url(cx))),
                        )
                        .child(
                            IconButton::new("x")
                                .variant(ButtonVariant::Ghost)
                                .size(gpui::px(32.0))
                                .icon_size(gpui::px(13.0))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.attachment_url_open = false;
                                    this.attachment_url_state
                                        .update(cx, |state, cx| state.reset(cx));
                                    cx.notify();
                                })),
                        ),
                )
            })
    }

    fn render_ticket_delivery_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let is_note = self.compose_note;
        let trigger = composer_delivery_chip(
            "support-ticket-delivery",
            if is_note { "sticky-note" } else { "reply" },
            if is_note {
                t!("support.note_internal").to_string()
            } else {
                t!("support.compose.reply").to_string()
            },
            true,
        );
        let parent = cx.entity();

        Popover::new("support-ticket-delivery-popover")
            .anchor(Corner::BottomRight)
            .trigger(trigger)
            .content(move |window, cx| {
                let parent = parent.clone();
                cx.new(move |content_cx| {
                    PopoverContent::new(window, content_cx, move |_window, cx| {
                        let mut list = div().w(px(190.0)).flex().flex_col().gap(px(2.0));
                        for (index, (note, icon, label)) in [
                            (false, "reply", t!("support.compose.reply").to_string()),
                            (true, "sticky-note", t!("support.note_internal").to_string()),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let selected = is_note == note;
                            let row_parent = parent.clone();
                            list = list.child(
                                div()
                                    .id(("support-ticket-delivery-option", index))
                                    .h(px(32.0))
                                    .px(px(8.0))
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .rounded(px(7.0))
                                    .cursor_pointer()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_primary())
                                    .when(selected, |row| row.bg(ShellDeckColors::selected_bg()))
                                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                                    .child(lucide_icon(icon, 13.0, ShellDeckColors::text_muted()))
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
                                            row_parent.update(cx, |this, cx| {
                                                this.compose_note = note;
                                                cx.notify();
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

    pub(super) fn render_composer(
        &self,
        _ticket_id: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_note = self.compose_note;

        let placeholder = if is_note {
            t!("support.note_placeholder").to_string()
        } else {
            t!("support.compose.reply_placeholder").to_string()
        };

        let ai_enabled = self.ai_reply_enabled && !is_note;

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
                let empty = self.composer_state.read(cx).content().trim().is_empty();
                let send_entity = cx.entity();
                let mut frame = Composer::new("support-ticket-composer", &self.composer_state)
                    .placeholder(placeholder)
                    .min_rows(1)
                    .max_rows(7)
                    .commit(if is_note {
                        ComposerCommit::Labeled(t!("support.compose.add_note").to_string().into())
                    } else {
                        ComposerCommit::Send
                    })
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
                    )
                    .option(self.render_ticket_delivery_picker(cx));

                if ai_enabled {
                    frame = frame.action(
                        compact_composer_action(
                            "support-ai-reply",
                            "sparkles",
                            t!("ai.workflow.support_reply").to_string(),
                            true,
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                if let Some(id) = this.selected_id.clone() {
                                    cx.emit(SupportViewEvent::SuggestReply(id));
                                }
                            },
                        )),
                    );
                }
                frame
            })
            .when(self.attachment_panel_open, |composer| {
                composer.child(self.render_attachment_picker(cx))
            })
    }
}
