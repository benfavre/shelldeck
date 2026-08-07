use super::*;
use adabraka_ui::prelude::Composer;
use crate::icons::{ai_provider_inline, simple_icon};

impl SupportView {
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
            .child(
                IconButton::new("refresh")
                    .variant(ButtonVariant::Ghost)
                    .size(gpui::px(28.0))
                    .icon_size(gpui::px(12.0))
                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                        cx.emit(SupportViewEvent::IssuesRefresh);
                    })),
            );

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
            iss.title.clone()
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
                            .child(
                                div()
                                    .w(px(6.0))
                                    .h(px(6.0))
                                    .rounded_full()
                                    .bg(status_dot),
                            )
                            .child(issue_status_label(&iss.status)),
                    )
                    .when(iss.priority != "normal" && !iss.priority.trim().is_empty(), |el| {
                        el.child(div().flex_shrink_0().child(priority_badge(&iss.priority)))
                    })
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

    /// Chat-style bubble for one request comment. Mirrors the ticket bubble
    /// (`render_message`): per-line `max_w` on the body so long lines wrap
    /// with a Definite width (GPUI doesn't wrap otherwise), and the author's
    /// side of the thread — mine right, others left, notes flush left with a
    /// warning tint. Same containment fixes an earlier overlap where a wall
    /// of dashes in a description would bleed past the bubble border.
    pub(super) fn render_issue_comment(
        &self,
        c: &shelldeck_core::config::issues::IssueComment,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // System notes are of a different nature than a reply: `kind` on the
        // wire is one of "status" | "system" | "github". Each gets its own
        // tinted frame — stripping them all the way down would make a status
        // change indistinguishable from a message.
        if c.is_note() {
            return self.render_issue_note(c).into_any_element();
        }
        // Human reply: prose on the surface, no card, per the mockup's rule
        // "reading measure, not a form field".
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

        let mut head = div()
            .flex()
            .items_baseline()
            .flex_wrap()
            .gap(px(7.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(if author_matches_me {
                        ShellDeckColors::primary()
                    } else {
                        ShellDeckColors::text_primary()
                    })
                    .child(label),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.5))
                    .text_color(ShellDeckColors::text_muted())
                    .child(rel_time(c.at)),
            );
        // The channel the request came in on — same value as `iss.source`. Not
        // per-comment yet (the server does not split it that way), but the
        // slot exists in the layout so it will not need re-shuffling later.
        if let Some(chan) = self
            .issue_detail
            .as_ref()
            .and_then(|iss| Self::source_chip_label(&iss.source))
        {
            head = head.child(
                div()
                    .flex_shrink_0()
                    .h(px(18.0))
                    .px(px(6.0))
                    .rounded_full()
                    .bg(ShellDeckColors::bg_surface())
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(chan.to_string()),
            );
        }

        let mut bubble = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .max_w(px(560.0))
            .min_w(px(0.0))
            .child(head)
            .child(Self::render_body_lines(&c.body, ShellDeckColors::text_primary()));
        if !c.attachments.is_empty() {
            bubble = bubble.child(self.render_issue_attachment_links(&c.attachments, cx));
        }
        bubble.into_any_element()
    }

    /// One system note (`status`, `system` or `github`), rendered as the
    /// mockup's `.thr-note`: icon + one-line body + actor/time, in the frame
    /// colour that matches the kind.
    fn render_issue_note(
        &self,
        c: &shelldeck_core::config::issues::IssueComment,
    ) -> impl IntoElement {
        let (icon, border, bg) = match c.kind.as_str() {
            "status" => (
                "check",
                ShellDeckColors::primary().opacity(0.30),
                ShellDeckColors::primary().opacity(0.08),
            ),
            "github" => (
                "git-branch",
                ShellDeckColors::border(),
                ShellDeckColors::bg_surface(),
            ),
            _ => (
                "info",
                ShellDeckColors::warning().opacity(0.30),
                ShellDeckColors::warning().opacity(0.10),
            ),
        };
        let actor = if c.author.trim().is_empty() {
            None
        } else {
            Some(c.author.clone())
        };
        div()
            .flex()
            .items_start()
            .gap(px(10.0))
            .max_w(px(560.0))
            .min_w(px(0.0))
            .p(px(9.0))
            .rounded(px(7.0))
            .border_1()
            .border_color(border)
            .bg(bg)
            .child(
                div()
                    .flex_shrink_0()
                    .w(px(22.0))
                    .h(px(22.0))
                    .rounded(px(5.0))
                    .bg(ShellDeckColors::bg_primary())
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(lucide_icon(icon, 13.0, ShellDeckColors::text_muted())),
            )
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(px(11.5))
                    .line_height(relative(1.4))
                    .text_color(ShellDeckColors::text_primary())
                    .child(Self::render_body_lines(
                        &c.body,
                        ShellDeckColors::text_primary(),
                    ))
                    .child(
                        div()
                            .mt(px(2.0))
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(match actor {
                                Some(a) => format!("{} · {}", rel_time(c.at), a),
                                None => rel_time(c.at),
                            }),
                    ),
            )
    }

    /// Split a plain-text body into lines and stack them. `line_height` MUST
    /// sit on each child (a factor on the flex parent leaves the child boxes
    /// at raw font height and they overlap vertically — the exact defect the
    /// last release shipped).
    fn render_body_lines(body: &str, color: Hsla) -> impl IntoElement {
        let mut wrap = div()
            .flex()
            .flex_col()
            .w_full()
            .min_w(px(0.0))
            .text_size(px(12.5))
            .text_color(color);
        for line in body.split('\n') {
            let line = if line.is_empty() { " " } else { line };
            wrap = wrap.child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .line_height(relative(1.62))
                    .child(line.to_string()),
            );
        }
        wrap
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

        if self.issues_staff {
            let include_dispatch = !self.issue_instances.is_empty();
            items.extend(Self::staff_triage_items(
                iss,
                &id,
                include_dispatch,
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

    /// Staff-only triage entries (status / priority / assign-me / dispatch /
    /// GitHub sync-or-push) for the issue kebab. Split out so the guard is
    /// unambiguous — inlined, the closing `}` of the `if self.issues_staff`
    /// block was easy to misread as unconditional code.
    ///
    /// `include_dispatch` is a caller-side gate on `issue_instances` (only
    /// staff with at least one reachable runtime can dispatch).
    pub(super) fn staff_triage_items(
        iss: &Issue,
        id: &str,
        include_dispatch: bool,
        entity: &Entity<SupportView>,
    ) -> Vec<PopoverMenuItem> {
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

        let aid = id.to_string();
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

        if include_dispatch {
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
        let title: SharedString = self
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
            .unwrap_or_default()
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
        PopoverMenu::new(pos, items).on_close({
            let entity = entity.clone();
            move |_, cx| {
                entity.update(cx, |this, cx| {
                    this.close_issue_popover_menu(cx);
                });
            }
        })
    }

    pub(super) fn render_issue_header_subpanels(
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

    pub(super) fn render_issue_detail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(iss) = self.issue_detail.clone() else {
            return self.render_empty_issue_detail().into_any_element();
        };

        let assignee = assignee_display(&iss.assignee, None);
        // The badges ARE the triggers. The status and priority pickers already
        // existed — buried in the kebab, two clicks away (⋮ → Statut → choose).
        // The thing you want to change is right there on screen; making it the
        // button is the whole point of the mockup's `[• À traiter ⌄]`.
        // Non-staff still get plain badges: no chevron, no click.
        let staff = self.issues_staff;
        let mut meta_row = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(8.0))
            .child(
                div()
                    .id("iss-detail-status")
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .rounded(px(6.0))
                    .when(staff, |el| {
                        el.cursor_pointer()
                            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    })
                    .child(issue_status_badge(&iss.status))
                    .when(staff, |el| {
                        el.child(
                            svg()
                                .path(lucide_path("chevron-down"))
                                .size(px(11.0))
                                .text_color(ShellDeckColors::text_muted()),
                        )
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if !this.issues_staff {
                            return;
                        }
                        this.issue_status_menu = true;
                        this.issue_priority_menu_open = false;
                        this.issue_dispatch_menu = false;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("iss-detail-priority")
                    .flex()
                    .items_center()
                    .gap(px(3.0))
                    .rounded(px(6.0))
                    .when(staff, |el| {
                        el.cursor_pointer()
                            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    })
                    .child(priority_badge(&iss.priority))
                    .when(staff, |el| {
                        el.child(
                            svg()
                                .path(lucide_path("chevron-down"))
                                .size(px(11.0))
                                .text_color(ShellDeckColors::text_muted()),
                        )
                    })
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        if !this.issues_staff {
                            return;
                        }
                        this.issue_priority_menu_open = true;
                        this.issue_status_menu = false;
                        this.issue_dispatch_menu = false;
                        cx.notify();
                    })),
            )
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
        if let Some(label) = iss.site_label.as_ref().filter(|l| !l.trim().is_empty()) {
            meta_row = meta_row.child(Badge::new(label.clone()).variant(BadgeVariant::Outline));
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
                    .when(self.ai_issue_enabled, |row| {
                        let summary_id = iss.id.clone();
                        row.child(
                            Button::new("issue-ai-summary", "")
                                .variant(ButtonVariant::Ai)
                                .size(ButtonSize::Sm)
                                .tooltip(t!("ai.workflow.issue_summary").to_string())
                                .icon(IconSource::from("info"))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(SupportViewEvent::SummarizeIssue(summary_id.clone()));
                                })),
                        )
                    })
                    .when(self.ai_issue_enabled && self.issues_staff, |row| {
                        let triage_id = iss.id.clone();
                        row.child(
                            Button::new("issue-ai-triage", "")
                                .variant(ButtonVariant::Ai)
                                .size(ButtonSize::Sm)
                                .tooltip(t!("ai.workflow.issue_triage").to_string())
                                .icon(IconSource::from("flag"))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(SupportViewEvent::TriageIssue(triage_id.clone()));
                                })),
                        )
                    })
                    .child({
                        let entity = cx.entity();
                        let iid = iss.id.clone();
                        IconButton::new("ellipsis-vertical")
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
            // The opening message is prose, like every reply below it. It used
            // to be an adabraka `Card` with a framed author tag — which is why
            // stripping `render_issue_comment` alone left this one boxed: it
            // never went through that function.
            thread = thread.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(3.0))
                    .w_full()
                    .max_w(px(560.0))
                    .min_w(px(0.0))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(7.0))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(if iss.requested_by.trim().is_empty() {
                                        t!("support.issue.description").to_string()
                                    } else {
                                        iss.requested_by.clone()
                                    }),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(10.5))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(rel_time(iss.created_at)),
                            ),
                    )
                    .child({
                        // 12.5px / 1.62 come from the proto's `.msg-ai` — a
                        // reading measure, not a form field. `TextVariant::BodySmall`
                        // is 13px / 1.5, a hair off both.
                        let mut body = div()
                            .flex()
                            .flex_col()
                            .w_full()
                            .min_w(px(0.0))
                            .text_size(px(12.5))
                            .text_color(ShellDeckColors::text_primary());
                        for line in iss.body.split('\n') {
                            let line = if line.is_empty() { " " } else { line };
                            body = body.child(
                                div()
                                    .w_full()
                                    .min_w(px(0.0))
                                    .line_height(relative(1.62))
                                    .child(line.to_string()),
                            );
                        }
                        body
                    }),
            );
        } else if iss.comments.is_empty() && iss.attachments.is_empty() {
            thread = thread.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.empty.comments").to_string()),
            );
        }
        if !iss.attachments.is_empty() {
            thread = thread.child(self.render_issue_attachment_links(&iss.attachments, cx));
        }
        for c in &iss.comments {
            thread = thread.child(self.render_issue_comment(c, cx));
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

    /// The provider chip for the reply composer — same slot the assistant gives
    /// its model, same persistence route (Workspace → Settings).
    fn render_support_ai_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use shelldeck_core::ai::AiBackend;
        let open = self.ai_backend_menu;
        let model = if self.ai_model.trim().is_empty() {
            self.ai_backend.default_model().to_string()
        } else {
            self.ai_model.trim().to_string()
        };
        let mut wrap = div().relative().flex().flex_shrink_0().child(
            div()
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
                .child(ai_provider_inline(self.ai_backend, &model))
                .child(
                    svg()
                        .path(lucide_path(if open { "chevron-up" } else { "chevron-down" }))
                        .size(px(11.0))
                        .flex_shrink_0()
                        .text_color(ShellDeckColors::text_muted()),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.ai_backend_menu = !this.ai_backend_menu;
                    cx.notify();
                })),
        );
        if open {
            let current = self.ai_backend;
            let mut list = div()
                .id("sup-ai-backend-menu")
                .w(px(208.0))
                .p(px(4.0))
                .flex()
                .flex_col()
                .gap(px(1.0))
                .bg(ShellDeckColors::bg_surface())
                .border_1()
                .border_color(ShellDeckColors::border())
                .rounded(px(9.0))
                .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                    cx.stop_propagation()
                });
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
                list = list.child(
                    div()
                        .id(("sup-ai-opt", index))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(9.0))
                        .py(px(7.0))
                        .rounded(px(7.0))
                        .cursor_pointer()
                        .text_size(px(12.0))
                        .when(selected, |el| el.bg(ShellDeckColors::selected_bg()))
                        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                        .child(match backend {
                            AiBackend::ClaudeCli => {
                                simple_icon("claudecode", 14.0, ShellDeckColors::text_primary())
                                    .into_any_element()
                            }
                            AiBackend::CodexCli | AiBackend::OpenAi => {
                                simple_icon("openai", 14.0, ShellDeckColors::text_primary())
                                    .into_any_element()
                            }
                            AiBackend::Anthropic => {
                                simple_icon("anthropic", 14.0, ShellDeckColors::text_primary())
                                    .into_any_element()
                            }
                            _ => lucide_icon("terminal", 14.0, ShellDeckColors::text_primary())
                                .into_any_element(),
                        })
                        .child(div().flex_1().min_w(px(0.0)).child(label))
                        .when(selected, |el| {
                            el.child(lucide_icon("check", 13.0, ShellDeckColors::primary()))
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.ai_backend_menu = false;
                            cx.emit(SupportViewEvent::SelectAiBackend(backend));
                            cx.notify();
                        })),
                );
            }
            wrap = wrap.child(
                deferred(
                    anchored()
                        .position_mode(gpui::AnchoredPositionMode::Local)
                        // Upward, explicitly: this composer sits at the bottom
                        // of the panel, so there is never room below. Letting
                        // `snap_to_window` flip it produced a menu that landed
                        // back on top of its own chip, because the +32 drop was
                        // still applied after the flip.
                        .position(point(gpui::px(0.0), gpui::px(-6.0)))
                        .anchor(gpui::Corner::BottomLeft)
                        .child(list),
                )
                .with_priority(3),
            );
        }
        wrap
    }

    /// The AI reply card as `.thr-ai-draft` in the mockup — a proposal to
    /// review, not a keystroke. It sits above the composer so `Publier`
    /// prepends into the user's current text (whatever they had is preserved).
    fn render_issue_ai_draft_card(
        &self,
        body: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .p(px(11.0))
            .rounded(px(10.0))
            .border_1()
            .border_color(ShellDeckColors::primary().opacity(0.40))
            .bg(ShellDeckColors::primary().opacity(0.08))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.0))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::primary())
                    .child(lucide_icon("sparkles", 12.0, ShellDeckColors::primary()))
                    .child(t!("support.issue.ai_draft").to_string()),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .line_height(relative(1.55))
                    .text_color(ShellDeckColors::text_primary())
                    .child(Self::render_body_lines(&body, ShellDeckColors::text_primary())),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().flex_1())
                    .child(
                        Button::new("issue-ai-discard", t!("support.issue.ai_discard").to_string())
                            .variant(ButtonVariant::Ghost)
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(SupportViewEvent::DiscardIssueAiDraft);
                            })),
                    )
                    .child(
                        Button::new("issue-ai-publish", t!("support.issue.ai_publish").to_string())
                            .variant(ButtonVariant::Ai)
                            .size(ButtonSize::Sm)
                            .icon(IconSource::from("arrow-up"))
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.emit(SupportViewEvent::PublishIssueAiDraft);
                            })),
                    ),
            )
    }

    pub(super) fn render_issue_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = adabraka_ui::theme::use_theme();
        let issue_id = self.issue_selected.clone();
        let ai_draft_card = self
            .issue_ai_draft
            .as_ref()
            .map(|draft| self.render_issue_ai_draft_card(draft.body.clone(), cx));
        div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .px(px(14.0))
            .py(px(10.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .on_action(cx.listener(|this, _: &EditorPaste, _, cx| {
                if this.paste_attachment(cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .children(ai_draft_card)
            .when(self.ai_issue_enabled && issue_id.is_some(), |composer| {
                let issue_id = issue_id.clone().unwrap_or_default();
                composer.child(div().h(px(0.0)).child(
                    // Kept as a marker so the `when` arm still has a body; the
                    // AI action now lives in the composer footer below.
                    div().id(SharedString::from(format!("issue-ai-anchor-{issue_id}"))),
                ))
            })
            .child({
                // The shared composer, hosting an `Editor` rather than an
                // `Input`: Support writes replies, not one-liners. Everything
                // that used to sit loose around the field — the AI suggestion
                // above it, the attachment toggle and Send below — is now in the
                // frame's footer, in the order every other surface uses.
                let focus = self.composer_state.read(cx).focus_handle(cx);
                let send_entity = cx.entity();
                let ai_issue_id = issue_id.clone().unwrap_or_default();
                let empty = self.composer_state.read(cx).content().trim().is_empty();
                let mut frame = Composer::with_field(
                    "sup-issue-composer",
                    focus,
                    div()
                        .w_full()
                        .min_w(px(0.0))
                        // Two lines, not four: the field grows with what you
                        // write instead of reserving an empty box.
                        .h(px(54.0))
                        // Vertical breathing room. Without it the caret sits
                        // flush against the frame's top border — the collision
                        // `.agents/spacing.md` calls blocking.
                        .pt(px(8.0))
                        .pb(px(4.0))
                        .px(px(8.0))
                        .overflow_hidden()
                        .child(
                            // `show_border(false)`: the `Composer` frame owns
                            // the border and the focus ring. Left on, the editor
                            // drew a second frame inside the first — the exact
                            // thing this component exists to prevent.
                            Editor::new(&self.composer_state)
                                .placeholder(t!("support.issue_comment_placeholder").to_string())
                                .font_family(theme.tokens.font_family.clone())
                                .show_border(false)
                                .min_lines(2)
                                .max_lines(2)
                                .show_horizontal_scrollbar(false)
                                .current_line_color(transparent_black()),
                        ),
                )
                // Grey while there is nothing to send, like every other
                // composer in the app.
                .commit_enabled(!self.attachment_busy && !empty)
                .action(
                    // Icon only. A bordered "Images jointes" button next to a
                    // plain-text AI action made two different kinds of control
                    // in the same footer row.
                    Button::new("issue-attachments-toggle", "")
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .selected(self.attachment_panel_open)
                        .tooltip(t!("user.requests.attachments.title").to_string())
                        .icon(IconSource::from("plus"))
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
                    // Hand-rolled rather than an adabraka `Button`: `ButtonSize::Sm`
                    // is a fixed 36px with medium weight, which towered over the
                    // 26px controls beside it. The footer's own scale is 11.5px
                    // muted (see `.agents/chrome.md` on adabraka's absolute sizes).
                    frame = frame.action(
                        div()
                            .id("issue-ai-reply")
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .h(px(26.0))
                            .px(px(6.0))
                            .rounded(px(7.0))
                            .cursor_pointer()
                            .text_size(px(11.5))
                            .text_color(ShellDeckColors::text_muted())
                            .hover(|style| {
                                style
                                    .bg(ShellDeckColors::hover_bg())
                                    .text_color(ShellDeckColors::text_primary())
                            })
                            .child(
                                svg()
                                    .path(lucide_path("sparkles"))
                                    .size(px(14.0))
                                    .flex_shrink_0()
                                    .text_color(ShellDeckColors::text_muted()),
                            )
                            .child(t!("ai.workflow.issue_reply").to_string())
                            .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                                cx.emit(SupportViewEvent::SuggestIssueReply(ai_issue_id.clone()));
                            })),
                    );
                    // The right-hand slot — where the assistant puts its model —
                    // holds the provider here too.
                    frame = frame.option(self.render_support_ai_picker(cx));
                }
                frame
            })
            .when(self.attachment_panel_open, |composer| {
                composer.child(self.render_attachment_picker(cx))
            })
    }
}
