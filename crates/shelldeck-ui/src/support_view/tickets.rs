use super::*;

impl SupportView {
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
            t.subject.clone()
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
            .opacity(0.35)
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

        // Line 1: channel glyph + subject + priority dot + time + kebab
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
                .child(lucide_icon(
                    t.channel_lucide(),
                    12.0,
                    ShellDeckColors::text_muted(),
                ))
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
                .child(priority_badge(&t.priority))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(rel_time(t.last_at)),
                )
                .child(kebab),
        );
        // Line 2: contact only. Message previews made compact virtualized rows
        // visually unstable at narrow widths; the full message remains in the
        // selected ticket detail.
        row = row.child(
            div().flex().items_center().child(
                div()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t.contact.display()),
            ),
        );
        row
    }

    pub(super) fn render_message(
        &self,
        msg: &SupportMessage,
        me: &SupportMe,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (bg, align_end, label) = if msg.is_note() {
            (
                ShellDeckColors::warning().opacity(0.12),
                false,
                t!("support.note_internal").to_string(),
            )
        } else if msg.is_customer() {
            (
                ShellDeckColors::bg_surface(),
                false,
                t!("support.bubble.client").to_string(),
            )
        } else {
            (
                ShellDeckColors::primary().opacity(0.12),
                true,
                t!("support.bubble.agent").to_string(),
            )
        };
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
                    let name = me.name.trim();
                    if !name.is_empty() {
                        Some(name.to_string())
                    } else {
                        let email = me.email.trim();
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
            .unwrap_or(label);

        // Bubble: `max_w(560)` caps the pill width; leaving the width
        // otherwise unconstrained lets the flex parent (`justify_end` on
        // the wrap when this is an agent-side message) push the bubble to
        // the correct edge. `min_w_0` + `w_full` on the text child were
        // added earlier to force horizontal wrap, but they made the bubble
        // stretch past its cap and broke the right-alignment for agent
        // messages — reverted to the pre-SDPATCH-011-hotfix layout.
        let mut bubble = div()
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
                            .flex_shrink_0()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(rel_time(msg.at)),
                    ),
            )
            .child({
                // Split by hard newlines and give each line its own div
                // with a `max_w`. gpui's text element uses
                // `available_space.width` as `wrap_width` when the parent
                // constrains it; `max_w` on a per-line wrapper feeds a
                // Definite width down to `shape_text` so long lines wrap
                // to the right height, while short lines' wrappers stay
                // as narrow as their content. Result: bubble auto-sizes
                // to the widest actual line, capped at max_w, with a
                // correct measured height (no more bleed past the border).
                let mut body = div()
                    .flex()
                    .flex_col()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary());
                for line in msg.text.split('\n') {
                    let display: SharedString = if line.is_empty() {
                        " ".into()
                    } else {
                        line.to_string().into()
                    };
                    body = body.child(div().max_w(px(540.0)).child(display));
                }
                body
            });
        if !msg.attachments.is_empty() {
            bubble = bubble.child(self.render_issue_attachment_links(&msg.attachments, cx));
        }

        let mut wrap = div().w_full().flex();
        if align_end {
            wrap = wrap.justify_end();
        }
        wrap.child(bubble)
    }

    pub(super) fn action_button(
        &self,
        id: &'static str,
        label: String,
        icon: Option<&'static str>,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let mut btn = div()
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
            .hover(|s| s.bg(ShellDeckColors::hover_bg()));
        if let Some(icon_name) = icon {
            btn = btn
                .flex()
                .items_center()
                .gap(px(4.0))
                .child(lucide_icon(icon_name, 12.0, ShellDeckColors::text_muted()))
                .child(label);
        } else {
            btn = btn.child(label);
        }
        btn.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| on_click(this, cx)))
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
            items.push(
                PopoverMenuItem::new("menu-priority", t!("support.menu.priority").to_string())
                    .icon("flag")
                    .on_click({
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_popover_menu(cx);
                                this.priority_menu_open = true;
                                this.assign_menu_open = false;
                                cx.notify();
                            });
                        }
                    }),
            );
            items.push(
                PopoverMenuItem::new("menu-assign", t!("support.menu.assign").to_string())
                    .icon("users")
                    .on_click({
                        let entity = entity.clone();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.close_popover_menu(cx);
                                this.assign_menu_open = true;
                                this.priority_menu_open = false;
                                cx.notify();
                            });
                        }
                    }),
            );
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
                    .child(
                        div()
                            .text_size(px(22.0))
                            .text_color(ShellDeckColors::primary())
                            .child("💬"),
                    ),
            )
            .child(
                div()
                    .text_size(px(14.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(t!("support.empty.tickets").to_string()),
            )
            .child(
                div()
                    .max_w(px(320.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.empty.tickets_hint").to_string()),
            )
    }

    pub(super) fn render_conversation(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(ticket) = self.detail.clone() else {
            return self.render_empty_conversation();
        };
        let tid = ticket.id.clone();

        // Header — context card. Big subject, then a single meta row with the
        // contact avatar + name, the status + priority as color-coded Badges,
        // the assignee in plain French, and the "last activity" time. Aim is
        // that a non-tech agent can read the whole context in ~2 seconds.
        let contact_name = ticket.contact.display();
        let assignee = assignee_display(&ticket.assignee, Some(self.my_email()));
        let last_at = ticket.last_at;
        let subject = if ticket.subject.trim().is_empty() {
            t!("support.empty.no_subject").to_string()
        } else {
            ticket.subject.clone()
        };

        let meta_row = div()
            .flex()
            .items_center()
            .flex_wrap()
            .gap(px(8.0))
            .child(
                Avatar::new()
                    .name(contact_name.clone())
                    .size(AvatarSize::Xs),
            )
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_primary())
                    .child(contact_name),
            )
            .child(status_badge(&ticket.status))
            .child(priority_badge(&ticket.priority))
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.assigned_to", name = assignee).to_string()),
            );
        let mut meta_row = meta_row;
        if last_at > 0.0 {
            meta_row = meta_row.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.last_exchange", time = rel_time(last_at)).to_string()),
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
                            .child(subject),
                    )
                    .child({
                        let entity = cx.entity();
                        let summary_id = tid.clone();
                        let triage_id = tid.clone();
                        let mut actions = div().flex().items_center().flex_shrink_0().gap(px(6.0));
                        if self.ai_reply_enabled {
                            actions = actions
                                .child(
                                    Button::new("support-ai-summary", "")
                                        .variant(ButtonVariant::Ai)
                                        .size(ButtonSize::Sm)
                                        .tooltip(t!("ai.workflow.support_summary").to_string())
                                        .icon(IconSource::from("info"))
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.emit(SupportViewEvent::SummarizeTicket(
                                                summary_id.clone(),
                                            ));
                                        })),
                                )
                                .child(
                                    Button::new("support-ai-triage", "")
                                        .variant(ButtonVariant::Ai)
                                        .size(ButtonSize::Sm)
                                        .tooltip(t!("ai.workflow.support_triage").to_string())
                                        .icon(IconSource::from("flag"))
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.emit(SupportViewEvent::TriageTicket(
                                                triage_id.clone(),
                                            ));
                                        })),
                                );
                        }
                        actions.child(
                            IconButton::new("ellipsis-vertical")
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
            .child(meta_row)
            .child(self.render_header_subpanels(&ticket, cx));

        // Messages (scrollable). Subtle background tint so the thread reads
        // as a distinct "conversation surface", separate from the white
        // header + action bar chrome. `bg_surface` is the same token adabraka
        // uses for card bodies — light-mode = warm cream, dark-mode = darker
        // panel, so the contrast stays gentle in both themes. `track_scroll`
        // wires the ScrollHandle that `set_detail` calls `scroll_to_bottom`
        // on, so opening a ticket lands on the newest message.
        let mut messages = div()
            .id("support-messages")
            .flex_1()
            // `min_h_0` on a flex_1 child is what actually lets the pane
            // shrink below its content height and enable overflow_y_scroll;
            // without it the tall content pushes the whole conversation
            // column past the composer.
            .min_h(px(0.0))
            .overflow_y_scroll()
            .track_scroll(&self.messages_scroll)
            .flex()
            .flex_col()
            .gap(px(8.0))
            .px(px(14.0))
            .pt(px(14.0))
            // Extra bottom padding so scroll_to_bottom leaves visible air
            // between the last bubble and the action bar's top border.
            .pb(px(20.0))
            .bg(ShellDeckColors::bg_surface());
        if ticket.messages.is_empty() {
            messages = messages.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("support.empty.messages").to_string()),
            );
        } else {
            for m in &ticket.messages {
                messages = messages.child(self.render_message(m, &self.me, cx));
            }
        }

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
            .child(header)
            .child(messages)
            .child(self.render_composer(&tid, cx))
    }

    /// Priority / assignee pickers opened from the header kebab menu.
    pub(super) fn render_header_subpanels(
        &self,
        ticket: &SupportTicket,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        if !self.priority_menu_open && !self.assign_menu_open {
            return div().into_any_element();
        }

        let id = ticket.id.clone();
        let mut panel = div().flex().flex_col().gap(px(6.0)).pt(px(4.0));

        if self.priority_menu_open {
            let mut prio_row = div()
                .w_full()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(6.0));
            for p in ["low", "normal", "high", "urgent"] {
                let pid = id.clone();
                let active = ticket.priority == p;
                let mut chip = div()
                    .id(ElementId::from(SharedString::from(format!(
                        "sup-pchip-{p}"
                    ))))
                    .p(px(2.0))
                    .rounded_full()
                    .cursor_pointer()
                    .border_2()
                    .child(priority_badge(p))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.priority_menu_open = false;
                        cx.emit(SupportViewEvent::SetPriority {
                            id: pid.clone(),
                            priority: p.to_string(),
                        });
                    }));
                if active {
                    chip = chip.border_color(ShellDeckColors::primary());
                } else {
                    chip = chip.border_color(gpui::transparent_black()).opacity(0.55);
                }
                prio_row = prio_row.child(chip);
            }
            panel = panel.child(prio_row);
        }

        if self.assign_menu_open {
            let mut list = div()
                .id("sup-assign-list")
                .w_full()
                .max_h(px(160.0))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(2.0));
            {
                let uid = id.clone();
                list = list.child(self.action_button(
                    "sup-unassign",
                    "— Non attribué —".to_string(),
                    Some("user"),
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
                let display_name = if agent.name.trim().is_empty() {
                    agent.email.clone()
                } else {
                    agent.name.clone()
                };
                let email_below = if agent.name.trim().is_empty() {
                    String::new()
                } else {
                    agent.email.clone()
                };
                let mut row = div()
                    .id(ElementId::from(SharedString::from(format!(
                        "sup-ag-{}",
                        agent.email
                    ))))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(9.0))
                    .py(px(5.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                    .child(
                        Avatar::new()
                            .name(display_name.clone())
                            .size(AvatarSize::Xs),
                    );
                let mut name_col = div().flex().flex_col().child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ShellDeckColors::text_primary())
                        .child(display_name),
                );
                if !email_below.is_empty() {
                    name_col = name_col.child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(email_below),
                    );
                }
                row = row.child(name_col).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        this.assign_menu_open = false;
                        cx.emit(SupportViewEvent::Assign {
                            id: aid.clone(),
                            assignee: email.clone(),
                        });
                    },
                ));
                list = list.child(row);
            }
            panel = panel.child(list);
        }

        panel.into_any_element()
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

    pub(super) fn render_attachment_toggle(
        &self,
        id: &'static str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let label = if self.attachment_drafts.is_empty() {
            t!("user.requests.attachments.title").to_string()
        } else {
            format!(
                "{} ({})",
                t!("user.requests.attachments.title"),
                self.attachment_drafts.len()
            )
        };
        Button::new(id, label)
            .size(ButtonSize::Sm)
            .variant(ButtonVariant::Outline)
            .selected(self.attachment_panel_open)
            .icon(IconSource::from("upload"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.attachment_panel_open = !this.attachment_panel_open;
                cx.notify();
            }))
    }

    pub(super) fn render_composer(
        &self,
        _ticket_id: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = adabraka_ui::theme::use_theme();
        let is_note = self.compose_note;
        let toggle =
            |label: &str, icon: &'static str, active: bool, note: bool, cx: &mut Context<Self>| {
                let color = if active {
                    ShellDeckColors::text_primary()
                } else {
                    ShellDeckColors::text_muted()
                };
                let mut b = div()
                    .id(ElementId::from(SharedString::from(format!(
                        "compose-mode-{note}"
                    ))))
                    .px(px(8.0))
                    .py(px(3.0))
                    .rounded(px(6.0))
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(lucide_icon(icon, 11.0, color))
                    .child(label.to_string())
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.compose_note = note;
                        cx.notify();
                    }));
                if active {
                    b = b.bg(ShellDeckColors::selected_bg()).text_color(color);
                } else {
                    b = b.text_color(color);
                }
                b
            };

        let placeholder = if is_note {
            t!("support.note_placeholder").to_string()
        } else {
            t!("support.compose.reply_placeholder").to_string()
        };

        let reply_label = t!("support.compose.reply").to_string();
        let note_label = t!("support.note_internal").to_string();
        let ai_enabled = self.ai_reply_enabled && !is_note;

        // 2-row composer: (1) mode toggle Réponse / Note interne (small
        // chips), (2) the Input widget flex_1 with the send button pinned
        // to its right so the reply flow reads as a single line. Previously
        // the send button sat on its own row below the Input, adding an
        // otherwise pointless third row of chrome.
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
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
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(toggle(&reply_label, "reply", !is_note, false, cx))
                    .child(toggle(&note_label, "sticky-note", is_note, true, cx))
                    .when(ai_enabled, |row| {
                        row.child(
                            Button::new(
                                "support-ai-reply",
                                t!("ai.workflow.support_reply").to_string(),
                            )
                            .variant(ButtonVariant::Ai)
                            .size(ButtonSize::Sm)
                            .icon(IconSource::from("sparkles"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(id) = this.selected_id.clone() {
                                    cx.emit(SupportViewEvent::SuggestReply(id));
                                }
                            })),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .h(px(80.0))
                            .overflow_hidden()
                            .child(
                                Editor::new(&self.composer_state)
                                    .placeholder(placeholder)
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
                            .child(self.render_attachment_toggle("support-attachments-toggle", cx))
                            .child(
                                Button::new(
                                    "support-send",
                                    if is_note {
                                        t!("support.compose.add_note").to_string()
                                    } else {
                                        t!("support.send").to_string()
                                    },
                                )
                                .variant(ButtonVariant::Default)
                                .size(ButtonSize::Sm)
                                .icon(IconSource::from("send"))
                                .disabled(self.attachment_busy)
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.send_composer(cx);
                                    },
                                )),
                            ),
                    ),
            )
    }
}
