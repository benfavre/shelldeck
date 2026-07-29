use super::*;

impl Workspace {
    /// Return a label that always carries a visible Unicode ellipsis when it
    /// exceeds the badge's known character budget. GPUI currently clips text
    /// nodes inside flex badges before painting its CSS-style ellipsis.
    pub(super) fn ellipsize_badge_label(label: &str, max_chars: usize) -> String {
        let mut chars = label.chars();
        let prefix: String = chars.by_ref().take(max_chars.saturating_sub(1)).collect();
        if chars.next().is_some() {
            format!("{prefix}…")
        } else {
            label.to_string()
        }
    }

    /// One row of the "Mes demandes" list — status badge, title, priority,
    /// optional GitHub number, and a hover-only red trash icon that opens
    /// the delete confirm. The hover kebab is hand-rolled (matches the
    /// sidebar's per-row action pattern) because adabraka `IconButton`
    /// derives its ElementId from the icon name and would collide across
    /// rows.
    pub(super) fn render_user_request_row(
        &self,
        iss: &Issue,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = iss.id.clone();
        let selected = self.issue_selected.as_deref() == Some(iss.id.as_str());
        let group_name = SharedString::from(format!("uiss-row-{}", iss.id));
        let mut row = div()
            .id(ElementId::from(SharedString::from(format!(
                "uiss-{}",
                iss.id
            ))))
            .group(group_name.clone())
            .w_full()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(10.0))
            .py(px(7.0))
            .rounded(px(8.0))
            .border_1()
            .border_color(if selected {
                ShellDeckColors::primary()
            } else {
                ShellDeckColors::border()
            })
            .cursor_pointer()
            .hover(|s| s.bg(ShellDeckColors::hover_bg()))
            .on_click({
                let id = id.clone();
                cx.listener(move |this, _: &ClickEvent, _, cx| this.select_issue(id.clone(), cx))
            })
            .child(issue_status_badge(&iss.status))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .truncate()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(iss.title.clone()),
            );
        if let Some(site_label) = iss
            .site_label
            .as_ref()
            .filter(|label| !label.trim().is_empty())
        {
            row = row.child(
                Badge::new(Self::ellipsize_badge_label(site_label, 17))
                    .variant(BadgeVariant::Outline)
                    .max_w(px(140.0))
                    .overflow_hidden(),
            );
        }
        row = row.child(priority_badge(&iss.priority));
        if let Some(g) = &iss.github {
            row = row.child(
                div()
                    .flex_shrink_0()
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("user.github_issue", number = g.number).to_string()),
            );
        }
        let del_id = iss.id.clone();
        row.child(
            div()
                .id(ElementId::from(SharedString::from(format!(
                    "uiss-del-{}",
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
                .text_color(ShellDeckColors::error())
                .opacity(0.0)
                .group_hover(group_name.clone(), |el| el.opacity(1.0))
                .hover(|el| el.bg(ShellDeckColors::error().opacity(0.15)))
                .child(
                    svg()
                        .path(lucide_path("trash-2"))
                        .size(px(13.0))
                        .text_color(ShellDeckColors::error()),
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.confirm_issue_delete = Some(del_id.clone());
                    cx.notify();
                })),
        )
    }

    /// User-mode "Mes demandes": a list of the tenant's requests. Selecting a
    /// row opens the detail as a right-side sheet; the "+ Nouvelle demande"
    /// button in the header opens the composer as another right-side sheet.
    /// Both live at the workspace root — they slide over the list without
    /// pushing it down (the pre-sheet layout used to append them inline).
    pub(super) fn render_user_requests(&self, cx: &mut Context<Self>) -> impl IntoElement {
        // User mode is the "as-a-normal-user" surface — even for a
        // super-admin viewing it, we only surface requests *they* filed.
        // (The server hands staff every in-scope request without a
        // `requested_by` filter — cf. `issuesInScope` in the manage repo — so
        // the "Mes demandes" label would otherwise be misleading.)
        let mine_count = self
            .issues_list
            .iter()
            .filter(|i| self.is_my_issue(i))
            .count();
        let list = if mine_count == 0 {
            div()
                .mt(px(8.0))
                .child(
                    div()
                        .py(px(8.0))
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("user.requests.empty").to_string()),
                )
                .into_any_element()
        } else {
            const MAX_LIST_H: f32 = 600.0;
            const MIN_LIST_H: f32 = 120.0;
            let visible_h = (mine_count as f32 * USER_REQUEST_ROW_H).clamp(MIN_LIST_H, MAX_LIST_H);
            div()
                .w_full()
                .h(px(visible_h))
                .mt(px(8.0))
                .child(
                    uniform_list(
                        "user-requests-virt",
                        mine_count,
                        cx.processor(|this, range: Range<usize>, _window, cx| {
                            let mine_indices = this
                                .issues_list
                                .iter()
                                .enumerate()
                                .filter(|(_, issue)| this.is_my_issue(issue))
                                .map(|(index, _)| index)
                                .collect::<Vec<_>>();
                            range
                                .filter_map(|index| mine_indices.get(index).copied())
                                .filter_map(|index| this.issues_list.get(index))
                                .map(|issue| {
                                    div()
                                        .w_full()
                                        .pb(px(4.0))
                                        .child(this.render_user_request_row(issue, cx))
                                        .into_any_element()
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .w_full()
                    .h_full(),
                )
                .into_any_element()
        };

        // Section header: title + "Nouvelle demande" button.
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .mb(px(4.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(lucide_icon("tag", 16.0, ShellDeckColors::text_muted()))
                    .child(
                        div()
                            .text_size(px(18.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(t!("user.requests.title").to_string()),
                    ),
            )
            .child(
                div()
                    .id("user-new-request-btn")
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::primary())
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(white())
                    .cursor_pointer()
                    .child(
                        svg()
                            .path("icons/lucide/plus.svg")
                            .size(px(11.0))
                            .text_color(white()),
                    )
                    .child(t!("user.requests.new").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.open_new_request(cx);
                    })),
            );

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .m(px(16.0))
            .child(header)
            .child(list)
    }

    /// Full-screen dimmed backdrop + right-anchored panel that wraps some inner
    /// content. Shared chrome for the two User-mode issue sheets (composer +
    /// detail). Clicking the backdrop or the header × triggers `on_close`;
    /// inner clicks are stopped so the backdrop doesn't dismiss.
    ///
    /// `dismissing = true` plays the exit animation (slide back off-screen
    /// right + fade out); `false` plays the enter animation.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_user_sheet<C: IntoElement + 'static>(
        &self,
        id: &'static str,
        title: String,
        icon: Option<&'static str>,
        dismissing: bool,
        inner: C,
        on_close: impl Fn(&mut Self, &mut Context<Self>) + Clone + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use std::time::Duration;
        const SHEET_WIDTH: f32 = 480.0;
        const ANIM_MS: u64 = SHEET_ANIM_MS;

        let close_bg = on_close.clone();
        div()
            .id(id)
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .bg(ShellDeckColors::backdrop())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _e, _window, cx| {
                    close_bg(this, cx);
                }),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .flex()
                    .flex_col()
                    .w(px(SHEET_WIDTH))
                    .bg(ShellDeckColors::bg_surface())
                    .border_l_1()
                    .border_color(ShellDeckColors::border())
                    .shadow_xl()
                    .overflow_hidden()
                    .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                        cx.stop_propagation();
                    })
                    // Sheet header: title + close button.
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .flex_shrink_0()
                            .px(px(20.0))
                            .py(px(14.0))
                            .border_b_1()
                            .border_color(ShellDeckColors::border())
                            .child({
                                let mut row = div().flex().items_center().gap(px(8.0));
                                if let Some(slug) = icon {
                                    row = row.child(lucide_icon(
                                        slug,
                                        16.0,
                                        ShellDeckColors::primary(),
                                    ));
                                }
                                row.child(
                                    div()
                                        .text_size(px(16.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(title.clone()),
                                )
                            })
                            .child({
                                let close = on_close.clone();
                                div()
                                    .id("user-sheet-close")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_color(ShellDeckColors::text_muted())
                                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                                    .child(
                                        svg()
                                            .path("icons/lucide/x.svg")
                                            .size(px(14.0))
                                            .text_color(ShellDeckColors::text_muted()),
                                    )
                                    .on_click(cx.listener(
                                        move |this, _: &ClickEvent, _window, cx| {
                                            close(this, cx);
                                        },
                                    ))
                            }),
                    )
                    // Body — scrollable if the content overflows the sheet.
                    .child(
                        div()
                            .id("user-sheet-body")
                            .flex_grow()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .p(px(16.0))
                            .child(inner),
                    )
                    // Slide (300ms). On enter: ease_out_quint (very smooth
                    // decel), from `right = -SHEET_WIDTH` to 0. On exit:
                    // ease_in_quint reversed. Encoding the direction in the
                    // id makes GPUI treat enter vs exit as distinct
                    // animations and restart cleanly on each flip.
                    .with_animation(
                        SharedString::from(format!(
                            "{id}-slide-{}",
                            if dismissing { "out" } else { "in" }
                        )),
                        Animation::new(Duration::from_millis(ANIM_MS)).with_easing(if dismissing {
                            (|t: f32| t * t * t * t * t) as fn(f32) -> f32 // ease_in_quint
                        } else {
                            (|t: f32| 1.0 - (1.0 - t).powi(5)) as fn(f32) -> f32
                            // ease_out_quint
                        }),
                        move |el, delta| {
                            let d = delta.clamp(0.0, 1.0);
                            let offset = if dismissing {
                                -SHEET_WIDTH * d
                            } else {
                                -SHEET_WIDTH * (1.0 - d)
                            };
                            el.right(gpui::px(offset))
                        },
                    ),
            )
    }

    /// The "Nouvelle demande" composer rendered as a right-side sheet.
    pub(super) fn render_issue_attachment_picker(
        &self,
        target: IssueAttachmentTarget,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let drafts = self.attachment_drafts(target).clone();
        let entity = cx.entity().downgrade();
        let previews =
            render_attachment_draft_gallery(&drafts, "issue-attachment-draft", move |index, cx| {
                if let Some(entity) = entity.upgrade() {
                    entity.update(cx, |this, cx| {
                        let drafts = this.attachment_drafts_mut(target);
                        if index < drafts.len() {
                            drafts.remove(index);
                        }
                        cx.notify();
                    });
                }
            });

        let url_input = Input::new(&self.issue_attachment_url_state)
            .size(InputSize::Sm)
            .placeholder(t!("user.requests.attachments.url_placeholder").to_string())
            .on_enter({
                let entity = cx.entity();
                move |_value, cx| {
                    entity.update(cx, |ws, cx| ws.import_issue_attachment_url(target, cx))
                }
            });

        div()
            .id(ElementId::from(SharedString::from(format!(
                "issue-attachment-picker-{target:?}"
            ))))
            .flex()
            .flex_col()
            .gap(px(8.0))
            .pt(px(9.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _, cx| {
                let mods = event.keystroke.modifiers;
                if event.keystroke.key.eq_ignore_ascii_case("v")
                    && (mods.control || mods.platform)
                    && this.paste_issue_attachment(target, cx)
                {
                    cx.stop_propagation();
                }
            }))
            .on_action(cx.listener(move |this, _: &Paste, _, cx| {
                if this.paste_issue_attachment(target, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, _, cx| {
                let generation = this.issue_attachment_generation;
                this.import_attachment_paths(target, paths.paths().to_vec(), generation, cx);
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
                                    drafts.len(),
                                    issues::ISSUE_ATTACHMENT_MAX_COUNT
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
                    .gap(px(6.0))
                    .child(
                        Button::new(
                            SharedString::from(format!("issue-file-{target:?}")),
                            t!("user.requests.attachments.file").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .icon(IconSource::from("upload"))
                        .disabled(self.issue_attachment_busy)
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.pick_issue_attachments(target, window, cx);
                            },
                        )),
                    )
                    .child(
                        Button::new(
                            SharedString::from(format!("issue-paste-{target:?}")),
                            t!("user.requests.attachments.paste").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .icon(IconSource::from("clipboard-paste"))
                        .disabled(self.issue_attachment_busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.paste_issue_attachment(target, cx) {
                                this.show_toast(
                                    t!("toast.issue.clipboard_no_image").to_string(),
                                    ToastLevel::Warning,
                                    cx,
                                );
                            }
                        })),
                    )
                    .child(
                        Button::new(
                            SharedString::from(format!("issue-capture-{target:?}")),
                            t!("user.requests.attachments.capture").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .icon(IconSource::from("scan"))
                        .disabled(self.issue_attachment_busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.capture_issue_attachment(target, cx);
                        })),
                    ),
            )
            .when(!drafts.is_empty(), |el| el.child(previews))
            .when(!self.issue_attachment_url_open, |el| {
                el.child(
                    Button::new(
                        SharedString::from(format!("issue-url-toggle-{target:?}")),
                        t!("user.requests.attachments.url_toggle").to_string(),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .icon(IconSource::from("globe"))
                    .disabled(self.issue_attachment_busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.issue_attachment_url_open = true;
                        cx.notify();
                    })),
                )
            })
            .when(self.issue_attachment_url_open, |el| {
                el.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .child(div().flex_1().min_w(px(0.0)).child(url_input))
                        .child(
                            Button::new(
                                SharedString::from(format!("issue-url-{target:?}")),
                                t!("user.requests.attachments.add_url").to_string(),
                            )
                            .size(ButtonSize::Sm)
                            .variant(ButtonVariant::Outline)
                            .icon(IconSource::from("globe"))
                            .disabled(self.issue_attachment_busy)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.import_issue_attachment_url(target, cx);
                                },
                            )),
                        )
                        .child(
                            IconButton::new("x")
                                .variant(ButtonVariant::Ghost)
                                .size(gpui::px(32.0))
                                .icon_size(gpui::px(13.0))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.issue_attachment_url_open = false;
                                    Self::reset_input(&this.issue_attachment_url_state.clone(), cx);
                                    cx.notify();
                                })),
                        ),
                )
            })
    }

    pub(super) fn render_stored_attachments(
        &self,
        attachments: &[issues::IssueAttachment],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity().downgrade();
        let lightbox_attachments = attachments.to_vec();
        let delete_entity = entity.clone();
        let delete_attachments = attachments.to_vec();
        let issue_id = self
            .issue_detail
            .as_ref()
            .map(|issue| issue.id.clone())
            .unwrap_or_default();
        render_stored_attachment_gallery(
            attachments,
            "stored-attachment",
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
                                    this.issue_attachment_lightbox = None;
                                    cx.notify();
                                });
                            }
                        },
                        cx,
                    )
                });
                entity.update(cx, |this, cx| {
                    this.issue_attachment_lightbox = Some(lightbox);
                    cx.notify();
                });
            },
            Some(Rc::new(move |index, cx| {
                let Some(attachment) = delete_attachments.get(index) else {
                    return;
                };
                if let Some(entity) = delete_entity.upgrade() {
                    entity.update(cx, |this, cx| {
                        this.confirm_attachment_delete =
                            Some((issue_id.clone(), attachment.id.clone()));
                        cx.notify();
                    });
                }
            })),
        )
    }

    pub(super) fn render_user_new_request_sheet(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let priorities = ["low", "normal", "high", "urgent"];
        let mut prio_row = div().flex().items_center().gap(px(6.0));
        for p in priorities {
            let active = self.issue_new_priority == p;
            // Colored adabraka Badge picks up the severity mapping; the
            // wrapper div carries the click-target + a soft ring on the
            // selected option so the picker still reads as a choice, not a
            // read-only tag.
            let mut chip = div()
                .id(ElementId::from(SharedString::from(format!(
                    "iss-np-sheet-{p}"
                ))))
                .p(px(2.0))
                .rounded_full()
                .cursor_pointer()
                .child(priority_badge(p))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.issue_new_priority = p.to_string();
                    cx.notify();
                }));
            if active {
                chip = chip.border_2().border_color(ShellDeckColors::primary());
            } else {
                chip = chip
                    .border_2()
                    .border_color(gpui::transparent_black())
                    .opacity(0.55);
            }
            prio_row = prio_row.child(chip);
        }

        // Real Input widgets — cursor, selection, undo, Enter to submit.
        // Sm size (32px h / 8px padx / 13px font) matches the compact look
        // the fake-input divs used before the migration.
        let title_input = Input::new(&self.issue_title_state)
            .size(InputSize::Sm)
            .placeholder(t!("user.requests.title_placeholder").to_string())
            .on_enter({
                let entity = cx.entity();
                move |_value, cx| {
                    entity.update(cx, |ws, cx| ws.submit_new_request(cx));
                }
            });
        let body_input = Input::new(&self.issue_body_state)
            .size(InputSize::Sm)
            .placeholder(t!("user.requests.body_placeholder").to_string())
            .multi_line(true)
            .min_rows(4);

        let ai_enabled = self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Issue);
        let mut inner = div().flex().flex_col().gap(px(10.0)).on_action(cx.listener(
            |this, _: &Paste, _, cx| {
                if this.paste_issue_attachment(IssueAttachmentTarget::NewRequest, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            },
        ));
        if ai_enabled {
            let model = if self.app_config.ai.model.trim().is_empty() {
                self.app_config.ai.backend.default_model().to_string()
            } else {
                self.app_config.ai.model.clone()
            };
            let expanded = self.issue_ai_expanded;
            let trigger = div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(8.0))
                .w_full()
                .px(px(10.0))
                .py(px(8.0))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .min_w(px(0.0))
                        .child(lucide_icon("sparkles", 14.0, ShellDeckColors::primary()))
                        .child(
                            div()
                                .truncate()
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(ShellDeckColors::primary())
                                .child(t!("user.requests.ai.title").to_string()),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .flex_shrink_0()
                        .child(ai_provider_badge(self.app_config.ai.backend, &model))
                        .child(
                            svg()
                                .path(lucide_path("chevron-down"))
                                .size(px(13.0))
                                .text_color(ShellDeckColors::text_muted())
                                .with_transformation(gpui::Transformation::rotate(gpui::radians(
                                    if expanded {
                                        0.0
                                    } else {
                                        -std::f32::consts::FRAC_PI_2
                                    },
                                ))),
                        ),
                );

            let mut content = div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .px(px(10.0))
                .pb(px(10.0))
                .child(
                    div()
                        .flex()
                        .items_end()
                        .gap(px(8.0))
                        .child(
                            div().flex_1().min_w(px(0.0)).child(
                                Input::new(&self.issue_ai_prompt_state)
                                    .size(InputSize::Sm)
                                    .multi_line(true)
                                    .min_rows(2)
                                    .max_rows(4)
                                    .placeholder(t!("user.requests.ai.placeholder").to_string())
                                    .disabled(self.issue_ai_loading),
                            ),
                        )
                        .child(
                            Button::new(
                                "user-request-ai-generate",
                                t!("user.requests.ai.generate").to_string(),
                            )
                            .variant(ButtonVariant::Ai)
                            .size(ButtonSize::Sm)
                            .min_w(px(104.0))
                            .disabled(self.issue_ai_loading)
                            .icon(IconSource::from("sparkles"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.generate_new_request_with_ai(cx);
                            })),
                        ),
                );
            if self.issue_ai_loading {
                content = content.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(animated_monolith(
                            "request-ai-thinking",
                            28.0,
                            MonolithMotion::Thinking,
                        ))
                        .child(animated_loading_text(
                            "request-ai-thinking-text",
                            t!("user.requests.ai.generating").to_string(),
                        )),
                );
            }
            if let Some(error) = &self.issue_ai_error {
                content = content.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::error())
                        .child(error.clone()),
                );
            }

            let entity = cx.entity();
            let mut ai_block = AnimatedCollapsible::new()
                .open(expanded)
                .show_icon(false)
                .trigger(trigger)
                .on_toggle(move |open, _, cx| {
                    entity.update(cx, |workspace, cx| {
                        workspace.issue_ai_expanded = open;
                        cx.notify();
                    });
                })
                .rounded(px(6.0))
                .border_1()
                .border_color(ShellDeckColors::primary().opacity(0.35))
                .bg(ShellDeckColors::primary().opacity(0.07));
            if expanded {
                ai_block = ai_block.content(content);
            }
            inner = inner.child(ai_block);
        }

        inner = inner
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(5.0))
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("user.requests.site_label").to_string()),
                    )
                    .child(self.issue_site_select.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(div().flex_1().min_w(px(0.0)).child(title_input))
                    .when(
                        self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Naming),
                        |row| {
                            row.child(
                                Button::new("request-ai-name", t!("ai.naming.action").to_string())
                                    .variant(ButtonVariant::Ai)
                                    .size(ButtonSize::Sm)
                                    .icon(IconSource::from("sparkles"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.open_ai_workflow(
                                            AiWorkflowTarget::EntityNaming {
                                                kind: AiNamingKind::Issue,
                                                target_id: "new-request".to_string(),
                                            },
                                            cx,
                                        );
                                    })),
                            )
                        },
                    ),
            )
            .child(body_input)
            .child(self.render_issue_attachment_picker(IssueAttachmentTarget::NewRequest, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .mt(px(4.0))
                    .child(prio_row)
                    .child(
                        div()
                            .id("iss-create")
                            .px(px(14.0))
                            .py(px(8.0))
                            .rounded(px(6.0))
                            .bg(ShellDeckColors::primary())
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white())
                            .cursor_pointer()
                            .child(t!("user.requests.create").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.submit_new_request(cx);
                            })),
                    ),
            );

        self.render_user_sheet(
            "user-new-request-sheet",
            t!("user.requests.new").to_string(),
            Some("plus"),
            self.user_new_request_sheet_dismissing,
            inner,
            |this, cx| this.close_new_request_sheet(cx),
            cx,
        )
    }

    /// The selected-request detail rendered as a right-side sheet.
    pub(super) fn render_user_issue_detail_sheet(
        &self,
        iss: Issue,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let inner = self.render_user_issue_detail(&iss, cx);
        self.render_user_sheet(
            "user-issue-detail-sheet",
            t!("user.requests.detail_title").to_string(),
            Some("tag"),
            self.user_issue_detail_dismissing,
            inner,
            |this, cx| this.close_user_issue_detail(cx),
            cx,
        )
    }

    pub(super) fn render_user_issue_detail(
        &self,
        iss: &Issue,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut thread = div().flex().flex_col().gap(px(6.0)).mt(px(8.0));
        if !iss.body.trim().is_empty() {
            thread = thread.child(
                div()
                    .p(px(10.0))
                    .rounded(px(8.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_primary())
                    .child(iss.body.clone()),
            );
        }
        if !iss.attachments.is_empty() {
            thread = thread.child(self.render_stored_attachments(&iss.attachments, cx));
        }
        for c in &iss.comments {
            thread = thread.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .p(px(9.0))
                    .rounded(px(8.0))
                    .bg(if c.is_note() {
                        ShellDeckColors::warning().opacity(0.10)
                    } else {
                        ShellDeckColors::bg_sidebar()
                    })
                    .child(
                        div()
                            .text_size(px(10.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_muted())
                            .child(if c.is_note() {
                                c.kind.clone()
                            } else {
                                c.author.clone()
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(c.body.clone()),
                    ),
            );
            if !c.attachments.is_empty() {
                thread = thread.child(self.render_stored_attachments(&c.attachments, cx));
            }
        }

        // Detail content flows directly inside the sheet chrome — no inner box
        // (bg / border / rounded) so the sheet reads as a single surface, not
        // "a card inside a card".
        div()
            .flex()
            .flex_col()
            .gap(px(8.0))
            .mt(px(10.0))
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                if this.paste_issue_attachment(IssueAttachmentTarget::Comment, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .child(
                div()
                    .flex()
                    .w_full()
                    .items_start()
                    .gap(px(8.0))
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .child(div().flex_shrink_0().child(issue_status_badge(&iss.status)))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .line_clamp(3)
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(ShellDeckColors::text_primary())
                            .child(iss.title.clone()),
                    )
                    .children(
                        iss.site_label
                            .as_ref()
                            .filter(|label| !label.trim().is_empty())
                            .map(|label| {
                                Badge::new(Self::ellipsize_badge_label(label, 13))
                                    .variant(BadgeVariant::Outline)
                                    .max_w(px(120.0))
                                    .flex_shrink_0()
                                    .overflow_hidden()
                            }),
                    )
                    .children(iss.github.as_ref().map(|g| {
                        div()
                            .id("uiss-gh")
                            .flex_shrink_0()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::primary())
                            .cursor_pointer()
                            .child(t!("user.github_issue", number = g.number).to_string())
                            .on_click({
                                let url = g.url.clone();
                                cx.listener(move |_t, _: &ClickEvent, _, _cx| {
                                    let _ = cloud_account::open_in_browser(&url);
                                })
                            })
                    })),
            )
            .child(thread)
            .child(self.render_issue_attachment_picker(IssueAttachmentTarget::Comment, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.0))
                    .child(
                        div().flex_1().child(
                            Input::new(&self.issue_comment_state)
                                .size(InputSize::Sm)
                                .placeholder(t!("user.requests.comment_placeholder").to_string())
                                .on_enter({
                                    let entity = cx.entity();
                                    move |_value, cx| {
                                        entity.update(cx, |ws, cx| ws.submit_issue_comment(cx));
                                    }
                                }),
                        ),
                    )
                    .child(
                        div()
                            .id("uiss-comment-send")
                            .flex()
                            .items_center()
                            .gap(px(6.0))
                            .px(px(12.0))
                            .py(px(7.0))
                            .rounded(px(6.0))
                            .bg(ShellDeckColors::primary())
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(white())
                            .cursor_pointer()
                            .child(
                                svg()
                                    .path(lucide_path("send"))
                                    .size(px(11.0))
                                    .text_color(white()),
                            )
                            .child(t!("user.requests.send").to_string())
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.submit_issue_comment(cx);
                            })),
                    ),
            )
            .when(self.is_my_issue(iss), |el| {
                el.child(
                    div().mt(px(8.0)).flex().justify_end().child(
                        Button::new("uiss-delete", t!("support.menu.delete").to_string())
                            .variant(ButtonVariant::Destructive)
                            .icon(IconSource::from("trash-2"))
                            .on_click({
                                let id = iss.id.clone();
                                cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.confirm_issue_delete = Some(id.clone());
                                    cx.notify();
                                })
                            }),
                    ),
                )
            })
    }
}
