use super::*;

impl SupportView {
    pub(super) fn render_home(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let stat = |icon: &'static str, value: usize, label: String, color: Hsla| {
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
                        ),
                )
                .min_w(px(180.0))
                .flex_1()
        };

        let entity = cx.entity();
        let tickets = Button::new(
            "support-home-tickets",
            t!("support.home.open_tickets").to_string(),
        )
        .icon(IconSource::from("inbox"))
        .on_click(move |_, _, cx| {
            entity.update(cx, |this, cx| {
                this.section = SupportSection::Tickets;
                cx.notify();
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
                this.section = SupportSection::Requests;
                cx.emit(SupportViewEvent::IssuesRefresh);
                cx.notify();
            });
        });

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
                        .child(stat(
                            "inbox",
                            self.counts.open as usize,
                            t!("support.home.open").to_string(),
                            ShellDeckColors::primary(),
                        ))
                        .child(stat(
                            "clock",
                            self.counts.breaching as usize,
                            t!("support.home.breaching").to_string(),
                            ShellDeckColors::error(),
                        ))
                        .child(stat(
                            "user-x",
                            self.counts.unassigned as usize,
                            t!("support.home.unassigned").to_string(),
                            ShellDeckColors::warning(),
                        ))
                        .child(stat(
                            "tag",
                            self.visible_issue_count(),
                            t!("support.home.requests").to_string(),
                            ShellDeckColors::success(),
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
                ),
        ))
    }
}
