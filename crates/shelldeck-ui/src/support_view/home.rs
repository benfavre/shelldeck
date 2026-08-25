use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SupportHomeTarget {
    AllTickets,
    OpenTickets,
    BreachingTickets,
    UnassignedTickets,
    Requests,
}

struct SupportHomeStat {
    id: &'static str,
    icon: &'static str,
    value: usize,
    label: String,
    color: Hsla,
    target: SupportHomeTarget,
}

impl SupportHomeTarget {
    fn section(self) -> SupportSection {
        match self {
            Self::Requests => SupportSection::Requests,
            _ => SupportSection::Tickets,
        }
    }

    fn ticket_filter(self) -> Option<SupportFilter> {
        match self {
            Self::AllTickets => Some(SupportFilter::All),
            Self::OpenTickets => Some(SupportFilter::Open),
            Self::BreachingTickets => Some(SupportFilter::Breaching),
            Self::UnassignedTickets => Some(SupportFilter::Unassigned),
            Self::Requests => None,
        }
    }
}

fn attention_rank(ticket: &SupportTicket) -> u8 {
    if ticket.sla.breaching || ticket.sla.breached {
        0
    } else if ticket.priority == "urgent" {
        1
    } else if ticket.is_unassigned() {
        2
    } else {
        3
    }
}

fn attention_ticket_indices(tickets: &[SupportTicket], limit: usize) -> Vec<usize> {
    let mut indices = tickets
        .iter()
        .enumerate()
        .filter(|(_, ticket)| ticket.status != "closed")
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    indices.sort_by(|left, right| {
        let left = &tickets[*left];
        let right = &tickets[*right];
        attention_rank(left)
            .cmp(&attention_rank(right))
            .then_with(|| right.last_at.total_cmp(&left.last_at))
    });
    indices.truncate(limit);
    indices
}

impl SupportView {
    fn open_home_target(&mut self, target: SupportHomeTarget, cx: &mut Context<Self>) {
        self.set_section(target.section());
        self.clear_selection();

        if let Some(filter) = target.ticket_filter() {
            self.filter = filter;
            self.search_state.update(cx, |state, cx| state.reset(cx));
            self.adv_channel = None;
            self.adv_priority = None;
            self.adv_unread_only = false;
            self.adv_assignee = None;
            self.adv_sla_only = false;
        } else {
            let filter = shelldeck_core::config::issues::IssueListFilter::default();
            self.issues_filter = filter.clone();
            self.issues_filter_draft = filter.clone();
            self.issues_search_state
                .update(cx, |state, cx| state.reset(cx));
            cx.emit(SupportViewEvent::IssuesFilterChanged { filter });
        }
        cx.notify();
    }

    fn render_home_stat(&self, stat: SupportHomeStat, cx: &mut Context<Self>) -> impl IntoElement {
        let SupportHomeStat {
            id,
            icon,
            value,
            label,
            color,
            target,
        } = stat;
        let entity = cx.entity();
        Card::new()
            .content(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .child(
                        div()
                            .size(px(38.0))
                            .rounded(px(10.0))
                            .bg(color.opacity(0.13))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(lucide_icon(icon, 18.0, color)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .text_size(px(24.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(value.to_string()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(label),
                            ),
                    )
                    .child(lucide_icon(
                        "chevron-right",
                        15.0,
                        ShellDeckColors::text_muted(),
                    )),
            )
            .min_w(px(180.0))
            .flex_1()
            .into_element()
            .id(id)
            .cursor_pointer()
            .hover(move |style| {
                style
                    .bg(ShellDeckColors::hover_bg())
                    .border_color(color.opacity(0.55))
            })
            .on_click(move |_, _, cx| {
                entity.update(cx, |this, cx| this.open_home_target(target, cx));
            })
    }

    fn render_home_ticket_row(
        &self,
        ticket: &SupportTicket,
        index: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = ticket.id.clone();
        let subject = if ticket.subject.trim().is_empty() {
            t!("support.empty.no_subject").to_string()
        } else {
            crate::external_content::external_title(&ticket.subject)
        };
        let mut meta = ticket.contact.display();
        let when = rel_time(ticket.last_at);
        if !when.is_empty() {
            meta.push_str(" · ");
            meta.push_str(&when);
        }
        let color = if ticket.sla.breaching || ticket.sla.breached {
            ShellDeckColors::error()
        } else if ticket.priority == "urgent" {
            ShellDeckColors::warning()
        } else {
            ShellDeckColors::primary()
        };

        div()
            .id(("support-home-ticket", index))
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(2.0))
            .py(px(9.0))
            .border_b_1()
            .border_color(ShellDeckColors::border().opacity(0.65))
            .cursor_pointer()
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                this.open_home_target(SupportHomeTarget::AllTickets, cx);
                cx.emit(SupportViewEvent::SelectTicket(id.clone()));
            }))
            .child(
                div()
                    .size(px(28.0))
                    .rounded(px(8.0))
                    .bg(color.opacity(0.11))
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_shrink_0()
                    .child(lucide_icon("inbox", 13.0, color)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(subject),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(10.5))
                            .text_color(ShellDeckColors::text_muted())
                            .child(meta),
                    ),
            )
            .when(
                ticket.priority != "normal" && !ticket.priority.trim().is_empty(),
                |row| {
                    row.child(
                        div()
                            .flex_shrink_0()
                            .child(priority_badge(&ticket.priority)),
                    )
                },
            )
            .child(lucide_icon(
                "chevron-right",
                13.0,
                ShellDeckColors::text_muted(),
            ))
    }

    fn render_home_issue_row(
        &self,
        issue: &Issue,
        index: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = issue.id.clone();
        let title = if issue.title.trim().is_empty() {
            t!("support.issue.no_title").to_string()
        } else {
            crate::external_content::external_title(&issue.title)
        };
        let when = rel_time(issue.updated_at);

        div()
            .id(("support-home-request", index))
            .flex()
            .items_center()
            .gap(px(10.0))
            .px(px(2.0))
            .py(px(9.0))
            .border_b_1()
            .border_color(ShellDeckColors::border().opacity(0.65))
            .cursor_pointer()
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if !event.standard_click() {
                    return;
                }
                this.open_home_target(SupportHomeTarget::Requests, cx);
                cx.emit(SupportViewEvent::SelectIssue(id.clone()));
            }))
            .child(
                div()
                    .size(px(28.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::success().opacity(0.11))
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_shrink_0()
                    .child(lucide_icon("tag", 13.0, ShellDeckColors::success())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .flex_1()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .truncate()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(title),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(10.5))
                            .text_color(ShellDeckColors::text_muted())
                            .child(format!("{} · {}", issue.tenant_name, when)),
                    ),
            )
            .child(issue_status_badge(&issue.status))
            .child(lucide_icon(
                "chevron-right",
                13.0,
                ShellDeckColors::text_muted(),
            ))
    }

    pub(super) fn render_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        let tickets = Button::new(
            "support-home-tickets",
            t!("support.home.open_tickets").to_string(),
        )
        .icon(IconSource::from("inbox"))
        .on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| {
                this.open_home_target(SupportHomeTarget::AllTickets, cx)
            });
        });
        let entity = cx.entity();
        let requests = Button::new(
            "support-home-requests",
            t!("support.home.open_requests").to_string(),
        )
        .variant(ButtonVariant::Outline)
        .icon(IconSource::from("tag"))
        .on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| {
                this.open_home_target(SupportHomeTarget::Requests, cx)
            });
        });

        let attention_indices = attention_ticket_indices(&self.tickets, 4);
        let attention = if attention_indices.is_empty() {
            div()
                .min_h(px(96.0))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.0))
                .text_color(ShellDeckColors::text_muted())
                .child(t!("support.empty.tickets_view").to_string())
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .children(
                    attention_indices
                        .into_iter()
                        .enumerate()
                        .map(|(row, index)| {
                            self.render_home_ticket_row(&self.tickets[index], row, cx)
                        }),
                )
                .into_any_element()
        };

        let mut recent_issue_indices = self
            .issues
            .iter()
            .enumerate()
            .filter(|(_, issue)| self.issues_staff || self.is_my_issue(issue))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        recent_issue_indices.sort_by(|left, right| {
            self.issues[*right]
                .updated_at
                .total_cmp(&self.issues[*left].updated_at)
        });
        recent_issue_indices.truncate(4);
        let recent_requests = if recent_issue_indices.is_empty() {
            div()
                .min_h(px(96.0))
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.0))
                .text_color(ShellDeckColors::text_muted())
                .child(t!("user.home.recent_empty").to_string())
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .children(
                    recent_issue_indices
                        .into_iter()
                        .enumerate()
                        .map(|(row, index)| {
                            self.render_home_issue_row(&self.issues[index], row, cx)
                        }),
                )
                .into_any_element()
        };

        div().flex_1().min_h(px(0.0)).child(scrollable_vertical(
            div()
                .flex()
                .flex_col()
                .gap(px(16.0))
                .p(px(20.0))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(4.0))
                        .child(
                            div()
                                .text_size(px(20.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(ShellDeckColors::text_primary())
                                .child(t!("support.home.title").to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(t!("support.home.subtitle").to_string()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(12.0))
                        .child(self.render_home_stat(
                            SupportHomeStat {
                                id: "support-home-open-stat",
                                icon: "inbox",
                                value: self.counts.open as usize,
                                label: t!("support.home.open").to_string(),
                                color: ShellDeckColors::primary(),
                                target: SupportHomeTarget::OpenTickets,
                            },
                            cx,
                        ))
                        .child(self.render_home_stat(
                            SupportHomeStat {
                                id: "support-home-sla-stat",
                                icon: "clock",
                                value: self.counts.breaching as usize,
                                label: t!("support.home.breaching").to_string(),
                                color: ShellDeckColors::error(),
                                target: SupportHomeTarget::BreachingTickets,
                            },
                            cx,
                        ))
                        .child(self.render_home_stat(
                            SupportHomeStat {
                                id: "support-home-unassigned-stat",
                                icon: "user-x",
                                value: self.counts.unassigned as usize,
                                label: t!("support.home.unassigned").to_string(),
                                color: ShellDeckColors::warning(),
                                target: SupportHomeTarget::UnassignedTickets,
                            },
                            cx,
                        ))
                        .child(self.render_home_stat(
                            SupportHomeStat {
                                id: "support-home-requests-stat",
                                icon: "tag",
                                value: self.visible_issue_count(),
                                label: t!("support.home.requests").to_string(),
                                color: ShellDeckColors::success(),
                                target: SupportHomeTarget::Requests,
                            },
                            cx,
                        )),
                )
                .child(
                    Card::new().content(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(12.0))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(4.0))
                                    .child(
                                        div()
                                            .text_size(px(14.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(ShellDeckColors::text_primary())
                                            .child(t!("support.home.priority_title").to_string()),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .text_color(ShellDeckColors::text_muted())
                                            .child(t!("support.home.priority_hint").to_string()),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.0))
                                    .child(requests)
                                    .child(tickets),
                            ),
                    ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_start()
                        .gap(px(12.0))
                        .child(
                            Card::new()
                                .header(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(7.0))
                                        .child(lucide_icon(
                                            "siren",
                                            15.0,
                                            ShellDeckColors::warning(),
                                        ))
                                        .child(
                                            div()
                                                .text_size(px(13.0))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(ShellDeckColors::text_primary())
                                                .child(
                                                    t!("support.home.priority_column_title")
                                                        .to_string(),
                                                ),
                                        ),
                                )
                                .content(attention)
                                .min_w(px(360.0))
                                .flex_1(),
                        )
                        .child(
                            Card::new()
                                .header(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(7.0))
                                        .child(lucide_icon(
                                            "history",
                                            15.0,
                                            ShellDeckColors::success(),
                                        ))
                                        .child(
                                            div()
                                                .text_size(px(13.0))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(ShellDeckColors::text_primary())
                                                .child(t!("user.home.recent_requests").to_string()),
                                        ),
                                )
                                .content(recent_requests)
                                .min_w(px(360.0))
                                .flex_1(),
                        ),
                ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{attention_ticket_indices, SupportHomeTarget};
    use crate::support_view::{SupportFilter, SupportSection};
    use shelldeck_core::config::manage_support::{SupportSla, SupportTicket};

    // SDTEST-1614
    #[test]
    fn support_home_targets_route_to_the_expected_section_and_ticket_filter() {
        let cases = [
            (
                SupportHomeTarget::AllTickets,
                SupportSection::Tickets,
                Some(SupportFilter::All),
            ),
            (
                SupportHomeTarget::OpenTickets,
                SupportSection::Tickets,
                Some(SupportFilter::Open),
            ),
            (
                SupportHomeTarget::BreachingTickets,
                SupportSection::Tickets,
                Some(SupportFilter::Breaching),
            ),
            (
                SupportHomeTarget::UnassignedTickets,
                SupportSection::Tickets,
                Some(SupportFilter::Unassigned),
            ),
            (SupportHomeTarget::Requests, SupportSection::Requests, None),
        ];

        for (target, section, filter) in cases {
            assert_eq!(target.section(), section);
            assert_eq!(target.ticket_filter(), filter);
        }
    }

    // SDTEST-1615
    #[test]
    fn support_home_attention_orders_sla_then_urgent_then_unassigned() {
        let ticket = |id: &str,
                      status: &str,
                      priority: &str,
                      assignee: &str,
                      breaching: bool,
                      last_at: f64| SupportTicket {
            id: id.to_string(),
            status: status.to_string(),
            priority: priority.to_string(),
            assignee: assignee.to_string(),
            last_at,
            sla: SupportSla {
                breaching,
                ..Default::default()
            },
            ..Default::default()
        };
        let tickets = vec![
            ticket(
                "ordinary",
                "open",
                "normal",
                "agent@example.com",
                false,
                50.0,
            ),
            ticket("unassigned", "open", "normal", "", false, 20.0),
            ticket("urgent", "open", "urgent", "agent@example.com", false, 10.0),
            ticket("sla", "open", "normal", "agent@example.com", true, 5.0),
            ticket("closed", "closed", "urgent", "", true, 100.0),
        ];

        let ids = attention_ticket_indices(&tickets, 4)
            .into_iter()
            .map(|index| tickets[index].id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(ids, vec!["sla", "urgent", "unassigned", "ordinary"]);
    }
}
