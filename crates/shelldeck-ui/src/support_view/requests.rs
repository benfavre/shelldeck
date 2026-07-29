use super::*;

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
                            let entity = cx.entity();
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

        row = row
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(lucide_icon("tag", 12.0, ShellDeckColors::text_muted()))
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
                    .child(issue_status_badge(&iss.status))
                    .child(priority_badge(&iss.priority))
                    .when(!when.is_empty(), |el| {
                        el.child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(when),
                        )
                    })
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
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(meta),
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
        let is_note = c.is_note();
        let author_matches_me = !c.author.trim().is_empty() && {
            let a = c.author.trim().to_ascii_lowercase();
            (!self.account_name_lc.is_empty() && a == self.account_name_lc)
                || (!self.account_email_lc.is_empty() && a == self.account_email_lc)
        };
        let (bg, align_end, label, icon) = if is_note {
            (
                ShellDeckColors::warning().opacity(0.12),
                false,
                if c.kind.is_empty() {
                    t!("support.issue.system").to_string()
                } else {
                    c.kind.clone()
                },
                "info",
            )
        } else if author_matches_me {
            (
                ShellDeckColors::primary().opacity(0.12),
                true,
                if c.author.trim().is_empty() {
                    t!("support.issue.comment").to_string()
                } else {
                    c.author.clone()
                },
                "reply",
            )
        } else {
            (
                ShellDeckColors::bg_surface(),
                false,
                if c.author.trim().is_empty() {
                    t!("support.issue.comment").to_string()
                } else {
                    c.author.clone()
                },
                "user",
            )
        };
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
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(lucide_icon(icon, 11.0, ShellDeckColors::text_muted()))
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(label),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(rel_time(c.at)),
                    ),
            )
            .child({
                let mut body = div()
                    .flex()
                    .flex_col()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary());
                for line in c.body.split('\n') {
                    let display: SharedString = if line.is_empty() {
                        " ".into()
                    } else {
                        line.to_string().into()
                    };
                    body = body.child(div().max_w(px(540.0)).child(display));
                }
                body
            })
            .when(!c.attachments.is_empty(), |el| {
                el.child(self.render_issue_attachment_links(&c.attachments, cx))
            });
        let mut wrap = div().w_full().flex();
        if align_end {
            wrap = wrap.justify_end();
        }
        wrap.child(bubble)
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
        let mut meta_row = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(8.0))
            .child(issue_status_badge(&iss.status))
            .child(priority_badge(&iss.priority))
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
            thread = thread.child(
                Card::new()
                    .header(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(px(10.0))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(4.0))
                                    .child(lucide_icon(
                                        "sticky-note",
                                        11.0,
                                        ShellDeckColors::text_muted(),
                                    ))
                                    .child(
                                        div()
                                            .text_size(px(10.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(ShellDeckColors::text_muted())
                                            .child(if iss.requested_by.trim().is_empty() {
                                                t!("support.issue.description").to_string()
                                            } else {
                                                iss.requested_by.clone()
                                            }),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(10.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(rel_time(iss.created_at)),
                            ),
                    )
                    .header_divider(false)
                    .content({
                        let mut body = div().flex().flex_col().w_full().min_w(px(0.0));
                        for line in iss.body.split('\n') {
                            let line = if line.is_empty() { " " } else { line };
                            body = body.child(
                                Text::new(line.to_string())
                                    .variant(TextVariant::BodySmall)
                                    .color(ShellDeckColors::text_primary())
                                    .line_height(1.35)
                                    .w_full(),
                            );
                        }
                        body
                    })
                    .w(px(560.0))
                    .max_w(relative(1.0))
                    .flex_shrink_0(),
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

    pub(super) fn render_issue_composer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = adabraka_ui::theme::use_theme();
        let entity = cx.entity();
        let issue_id = self.issue_selected.clone();
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
            .when(self.ai_issue_enabled && issue_id.is_some(), |composer| {
                let issue_id = issue_id.clone().unwrap_or_default();
                composer.child(
                    div().flex().items_center().pb(px(6.0)).child(
                        Button::new("issue-ai-reply", t!("ai.workflow.issue_reply").to_string())
                            .variant(ButtonVariant::Ai)
                            .size(ButtonSize::Sm)
                            .icon(IconSource::from("sparkles"))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(SupportViewEvent::SuggestIssueReply(issue_id.clone()));
                            })),
                    ),
                )
            })
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .h(px(80.0))
                    .overflow_hidden()
                    .child(
                        Editor::new(&self.composer_state)
                            .placeholder(t!("support.issue_comment_placeholder").to_string())
                            .font_family(theme.tokens.font_family.clone())
                            .min_lines(4)
                            .max_lines(4)
                            .show_horizontal_scrollbar(false)
                            .current_line_color(transparent_black()),
                    ),
            )
            .when(self.attachment_panel_open, |composer| {
                composer.child(self.render_attachment_picker(cx))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(self.render_attachment_toggle("issue-attachments-toggle", cx))
                    .child(
                        Button::new("sup-issue-send", t!("support.send").to_string())
                            .size(ButtonSize::Sm)
                            .h(gpui::px(32.0))
                            .icon(IconSource::from("send"))
                            .disabled(self.attachment_busy)
                            .on_click({
                                move |_, _, cx| {
                                    entity.update(cx, |this, cx| this.send_composer(cx));
                                }
                            }),
                    ),
            )
    }
}
