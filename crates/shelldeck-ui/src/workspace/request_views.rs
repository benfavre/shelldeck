use super::*;
use crate::overlay::window_backdrop;
use adabraka_ui::components::input::InputVariant;
use adabraka_ui::overlays::popover::PopoverContent;
use adabraka_ui::prelude::{Composer, ComposerCommit, Popover};
use shelldeck_core::ai::AiBackend;

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
        compact: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = iss.id.clone();
        let selected = self.issue_selected.as_deref() == Some(iss.id.as_str());
        let group_name = SharedString::from(format!("uiss-row-{}", iss.id));
        let row_h = if compact {
            USER_REQUEST_ROW_H_COMPACT
        } else {
            USER_REQUEST_ROW_H
        };
        let mut title = div()
            .min_w(px(0.0))
            .overflow_hidden()
            .text_size(px(13.0))
            .text_color(ShellDeckColors::text_primary());
        title = if compact {
            title.w_full().line_clamp(2)
        } else {
            title.flex_1().whitespace_nowrap().truncate()
        };
        let title = title.child(crate::external_content::external_title(&iss.title));

        let mut metadata = div().flex().items_center().min_w(px(0.0)).overflow_hidden();
        let mut metadata_has_item = compact;
        if compact {
            metadata = metadata.w_full().child(issue_status_badge(&iss.status));
        } else {
            metadata = metadata.flex_shrink_0();
        }
        if let Some(site_label) = iss
            .site_label
            .as_ref()
            .filter(|label| !label.trim().is_empty())
        {
            let item_margin = if metadata_has_item { 8.0 } else { 0.0 };
            metadata = metadata.child(
                div()
                    .flex()
                    .items_center()
                    .flex_shrink_0()
                    .ml(px(item_margin))
                    .child(
                        Badge::new(Self::ellipsize_badge_label(site_label, 17))
                            .variant(BadgeVariant::Outline)
                            .max_w(px(140.0))
                            .overflow_hidden(),
                    ),
            );
            metadata_has_item = true;
        }
        // Deux informations que la file Support affichait déjà et que le
        // client, propriétaire de la demande, n'avait pas : depuis quand elle
        // a bougé, et si quelqu'un y a répondu. Sans elles, on ne peut pas
        // savoir que le support a écrit.
        if iss.comment_count > 0 {
            let item_margin = if metadata_has_item { 8.0 } else { 0.0 };
            metadata = metadata.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .ml(px(item_margin))
                    .flex_shrink_0()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    // `messages-square` : le lot Lucide embarqué ne contient
                    // pas `message-square` au singulier, et un slug absent ne
                    // dessine rien du tout (`.agents/icons.md`).
                    .child(lucide_icon(
                        "messages-square",
                        11.0,
                        ShellDeckColors::text_muted(),
                    ))
                    .child(iss.comment_count.to_string()),
            );
            metadata_has_item = true;
        }
        let item_margin = if metadata_has_item { 8.0 } else { 0.0 };
        metadata = metadata.child(
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .ml(px(item_margin))
                .child(priority_badge(&iss.priority)),
        );
        metadata_has_item = true;
        let updated = crate::i18n::rel_time(iss.updated_at);
        if !updated.is_empty() {
            let item_margin = if metadata_has_item { 8.0 } else { 0.0 };
            metadata = metadata.child(
                div()
                    .flex_shrink_0()
                    .ml(px(item_margin))
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(updated),
            );
            metadata_has_item = true;
        }
        if let Some(g) = &iss.github {
            let item_margin = if metadata_has_item { 8.0 } else { 0.0 };
            metadata = metadata.child(
                div()
                    .flex_shrink_0()
                    .ml(px(item_margin))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("user.github_issue", number = g.number).to_string()),
            );
            metadata_has_item = true;
        }
        let del_id = iss.id.clone();
        let mut delete = div()
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
            }));
        if compact {
            delete = delete.ml_auto();
        } else if metadata_has_item {
            delete = delete.ml(px(8.0));
        }
        metadata = metadata.child(delete);

        let mut row = div()
            .id(ElementId::from(SharedString::from(format!(
                "uiss-{}",
                iss.id
            ))))
            .group(group_name.clone())
            .w_full()
            // The virtualized wrapper reserves the final 4 px as the gap to
            // the next card. Use its remaining height explicitly so internal
            // top/bottom padding stays symmetrical.
            .h(px(row_h - 4.0))
            .flex()
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
            });
        if compact {
            row = row.flex_col().justify_center().gap(px(5.0));
            row.child(title).child(metadata)
        } else {
            row = row.items_center().gap(px(8.0));
            row.child(issue_status_badge(&iss.status))
                .child(title)
                .child(metadata)
        }
    }

    /// User-mode "Mes demandes": a list of the tenant's requests. Selecting a
    /// row opens the detail as a right-side sheet; the "+ Nouvelle demande"
    /// button in the header opens the composer as another right-side sheet.
    /// Both live at the workspace root — they slide over the list without
    /// pushing it down (the pre-sheet layout used to append them inline).
    pub(super) fn render_user_requests(
        &self,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // User mode is the "as-a-normal-user" surface — even for a
        // super-admin viewing it, we only surface requests *they* filed.
        // (The server hands staff every in-scope request without a
        // `requested_by` filter — cf. `issuesInScope` in the manage repo — so
        // the "Mes demandes" label would otherwise be misleading.)
        let mine_count = self
            .issues_list
            .iter()
            .filter(|i| self.is_user_visible_issue(i))
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
            let request_row_h = if compact {
                USER_REQUEST_ROW_H_COMPACT
            } else {
                USER_REQUEST_ROW_H
            };
            let visible_h = (mine_count as f32 * request_row_h).clamp(MIN_LIST_H, MAX_LIST_H);
            div()
                .w_full()
                .h(px(visible_h))
                .mt(px(8.0))
                .child(
                    uniform_list(
                        "user-requests-virt",
                        mine_count,
                        cx.processor(move |this, range: Range<usize>, _window, cx| {
                            let mine_indices = this
                                .issues_list
                                .iter()
                                .enumerate()
                                .filter(|(_, issue)| this.is_user_visible_issue(issue))
                                .map(|(index, _)| index)
                                .collect::<Vec<_>>();
                            range
                                .filter_map(|index| mine_indices.get(index).copied())
                                .filter_map(|index| this.issues_list.get(index))
                                .map(|issue| {
                                    div()
                                        .w_full()
                                        // `uniform_list` positions slots by
                                        // the declared row height but does
                                        // not impose that height on the
                                        // element returned by the processor.
                                        // Without it, compact rows stretch
                                        // apart inside the reserved list.
                                        .h(px(request_row_h))
                                        .pb(px(4.0))
                                        .child(this.render_user_request_row(issue, compact, cx))
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
        is_maximized: bool,
        inner: C,
        footer: Option<AnyElement>,
        body_scroll: Option<&ScrollHandle>,
        on_close: impl Fn(&mut Self, &mut Context<Self>) + Clone + 'static,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        use std::time::Duration;
        const SHEET_WIDTH: f32 = 480.0;
        const ANIM_MS: u64 = SHEET_ANIM_MS;

        let close_bg = on_close.clone();
        let mut body = div()
            .id("user-sheet-body")
            .flex()
            .flex_col()
            .flex_grow()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .p(px(16.0));
        if let Some(scroll) = body_scroll {
            body = body.track_scroll(scroll);
        }
        body = body.child(inner);

        // Le rayon de la fenêtre est porté par la couche qui peint réellement
        // le fond, jamais par un ancêtre : voir `crate::overlay`.
        window_backdrop(id, is_maximized)
            // Leave the client-side titlebar outside the occluding layer: its
            // minimize / maximize / close controls must remain reachable while
            // a request is open. `window_backdrop` starts at zero and rounds
            // every corner, so override both the top edge and its now-internal
            // radii after constructing it.
            .top(px(WORKSPACE_TITLEBAR_HEIGHT))
            .rounded_t(px(0.0))
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
                    // This panel reaches the transparent window edge. GPUI
                    // paints an outer shadow beyond the rounded panel clip,
                    // which leaves a translucent square in the bottom-right
                    // corner; the left border already provides separation.
                    .overflow_hidden()
                    .map(|panel| {
                        if is_maximized {
                            panel
                        } else {
                            // The sheet begins below the titlebar, so its top
                            // edge is internal and square. It still owns the
                            // outer bottom-right corner it paints over.
                            panel.rounded_br(use_theme().tokens.radius_xl)
                        }
                    })
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
                    .child(body)
                    // Detail sheets keep their message composer outside the
                    // scroll body so replying never requires scrolling to the
                    // end of a long thread. Form sheets simply pass `None`.
                    .children(footer)
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
        separated: bool,
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

        let mut picker = div()
            .id(ElementId::from(SharedString::from(format!(
                "issue-attachment-picker-{target:?}"
            ))))
            .flex()
            .flex_col()
            .gap(px(8.0))
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
            });

        if separated {
            picker = picker
                .pt(px(9.0))
                .border_t_1()
                .border_color(ShellDeckColors::border());
        }
        picker
    }

    pub(super) fn render_stored_attachments(
        &self,
        attachments: &[issues::IssueAttachment],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity = cx.entity().downgrade();
        // Posted attachments become source-agnostic viewer items; the same
        // viewer also serves drafts held in memory (`LightboxItem::from_draft`).
        let lightbox_attachments: Vec<LightboxItem> =
            attachments.iter().map(LightboxItem::from_stored).collect();
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

    /// The AI provider chip, as a picker. Shared by the request sheet's AI
    /// panel header and by that panel's composer footer: the panel *moves* it
    /// when it expands. Collapsed, the header is the only place to show which
    /// backend will run; expanded, it belongs in the composer footer with the
    /// other settings — exactly where the assistant puts it.
    pub(super) fn render_ai_backend_picker(
        &self,
        model: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let open = self.issue_ai_backend_menu;
        let mut wrap = div().relative().flex().flex_shrink_0().child(
            div()
                .id("iss-ai-backend")
                .flex()
                .items_center()
                .gap(px(5.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                .child(ai_provider_badge(self.app_config.ai.backend, model))
                .child(
                    svg()
                        .path(lucide_path(if open {
                            "chevron-up"
                        } else {
                            "chevron-down"
                        }))
                        .size(px(12.0))
                        .flex_shrink_0()
                        .mr(px(2.0))
                        .text_color(ShellDeckColors::text_muted()),
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.issue_ai_backend_menu = !this.issue_ai_backend_menu;
                    cx.notify();
                })),
        );
        if open {
            let current = self.app_config.ai.backend;
            let mut list = div()
                .id("iss-ai-backend-menu")
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
                (AiBackend::AutomoniqueAcp, "Automonique ACP"),
                (AiBackend::OpenAi, "OpenAI API"),
                (AiBackend::Anthropic, "Anthropic API"),
            ]
            .into_iter()
            .enumerate()
            {
                let selected = backend == current;
                list = list.child(
                    div()
                        .id(("iss-ai-backend-opt", index))
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
                        // The provider marks, same as on the closed chip:
                        // recognising the Claude or OpenAI logo is faster than
                        // reading five near-identical CLI names.
                        .child(ai_provider_icon(
                            backend,
                            14.0,
                            ShellDeckColors::text_primary(),
                        ))
                        .child(div().flex_1().min_w(px(0.0)).child(label))
                        .when(selected, |el| {
                            el.child(lucide_icon("check", 13.0, ShellDeckColors::primary()))
                        })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.issue_ai_backend_menu = false;
                            // Routed through Settings: it
                            // owns `ai.*`.
                            this.settings.update(cx, |settings, cx| {
                                settings.set_ai_backend(backend, cx);
                            });
                            cx.notify();
                        })),
                );
            }
            wrap = wrap.child(
                deferred(
                    anchored()
                        .position_mode(gpui::AnchoredPositionMode::Local)
                        // The chip's left edge, like the site and priority
                        // pickers. A hardcoded −140 was tuned for the header
                        // and stayed wrong once the chip moved to the footer.
                        // 36, not 28: `ai_provider_badge` is a 30px badge,
                        // where the site and priority chips are 24px pills. The
                        // drop is chip height + 6, per surface — a shared
                        // constant here would touch one of the two.
                        .position(point(gpui::px(0.0), gpui::px(36.0)))
                        .snap_to_window_with_margin(gpui::px(8.0))
                        .anchor(gpui::Corner::TopLeft)
                        .child(list),
                )
                .with_priority(3),
            );
        }
        wrap
    }

    pub(super) fn render_user_new_request_sheet(
        &self,
        is_maximized: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // One chip, not four. Four badges plus a 186px site select overflowed
        // the context row onto a second line, and the unselected ones carried
        // `opacity(0.55)` — which made "Basse" unreadable on a light theme.
        // Hand-rolled, by exception. adabraka's `Select` derives its dropdown
        // width from its trigger, so a chip-sized trigger gives a chip-sized
        // menu with the site names clipped — and the popover needs to be wider
        // than the chip. `.agents/ui-components.md` asks for the reason to be
        // spelled out next to the divergence; this is it. The two pickers in
        // this row are therefore built from the same custom parts, which also
        // guarantees they stay identical.
        let site_label = self
            .issue_new_site_id
            .as_deref()
            .and_then(|id| {
                self.site_directory
                    .as_ref()
                    .and_then(|directory| directory.sites.iter().find(|site| site.site_id == id))
                    .map(|site| site.display_label())
            })
            .unwrap_or_else(|| t!("user.requests.site_none").to_string());
        let site_button = div()
            .id("iss-np-site")
            .flex()
            .items_center()
            .gap(px(6.0))
            .flex_shrink_0()
            .h(px(24.0))
            .max_w(px(200.0))
            .px(px(8.0))
            .rounded(px(6.0))
            .bg(ShellDeckColors::bg_surface())
            .text_size(px(11.0))
            .text_color(ShellDeckColors::text_primary())
            .cursor_pointer()
            .hover(|style| style.bg(ShellDeckColors::selected_bg()))
            .child(
                svg()
                    .path(lucide_path("globe"))
                    .size(px(12.0))
                    .flex_shrink_0()
                    .text_color(ShellDeckColors::text_muted()),
            )
            .child(div().min_w(px(0.0)).truncate().child(site_label))
            .child(
                svg()
                    .path(lucide_path(if self.issue_site_menu {
                        "chevron-up"
                    } else {
                        "chevron-down"
                    }))
                    .size(px(12.0))
                    .flex_shrink_0()
                    .text_color(ShellDeckColors::text_muted()),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.issue_site_menu = if this.issue_site_menu {
                    false
                } else {
                    // One picker at a time.
                    this.issue_priority_menu = false;
                    true
                };
                cx.notify();
            }));

        // The popover hangs off a plain wrapper, never off the chip: the chip
        // is a 24px-high flex row, and an anchored child inside it is measured
        // as one of its flex items instead of from the row's origin.
        let mut site_chip = div().relative().flex().flex_shrink_0().child(site_button);

        // Same skin as the compact site Select beside it — 24px pill, muted
        // fill, 11px label, chevron. The two chips do the same job (open a
        // picker) so they must look the same; a `Badge` inside a chip made one
        // a pill-in-a-pill and the other a plain badge with a loose chevron.
        // The severity colour survives as the leading dot; the menu still shows
        // the full coloured badges.
        let priority_dot = match self.issue_new_priority.as_str() {
            "urgent" => ShellDeckColors::error(),
            "high" => ShellDeckColors::warning(),
            "low" => ShellDeckColors::text_muted(),
            _ => ShellDeckColors::primary(),
        };
        let priority_chip = div()
            .id("iss-np-priority")
            .flex()
            .items_center()
            .gap(px(6.0))
            .flex_shrink_0()
            .h(px(24.0))
            .px(px(8.0))
            .rounded(px(6.0))
            .bg(ShellDeckColors::bg_surface())
            .text_size(px(11.0))
            .text_color(ShellDeckColors::text_primary())
            .cursor_pointer()
            .hover(|style| style.bg(ShellDeckColors::selected_bg()))
            .child(
                div()
                    .w(px(7.0))
                    .h(px(7.0))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(priority_dot),
            )
            .child(crate::support_view::priority_label(
                &self.issue_new_priority,
            ))
            .child(
                svg()
                    .path(lucide_path(if self.issue_priority_menu {
                        "chevron-up"
                    } else {
                        "chevron-down"
                    }))
                    .size(px(12.0))
                    .flex_shrink_0()
                    .text_color(ShellDeckColors::text_muted()),
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.issue_priority_menu = if this.issue_priority_menu {
                    false
                } else {
                    this.issue_site_menu = false;
                    true
                };
                cx.notify();
            }));

        let mut prio_row = div().relative().flex().flex_shrink_0().child(priority_chip);

        if self.issue_site_menu {
            let query = self.issue_site_search.read(cx).content().to_lowercase();
            let query = query.trim().to_string();
            let selected_id = self.issue_new_site_id.clone();
            let mut rows = div()
                .id("iss-np-site-rows")
                .flex()
                .flex_col()
                .gap(px(1.0))
                .max_h(px(260.0))
                .overflow_y_scroll();

            let none_selected = selected_id.is_none();
            rows = rows.child(
                div()
                    .id("iss-np-site-none")
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(8.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .cursor_pointer()
                    .text_size(px(12.0))
                    .when(none_selected, |el| el.bg(ShellDeckColors::selected_bg()))
                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    .child(
                        svg()
                            .path(lucide_path("globe"))
                            .size(px(13.0))
                            .flex_shrink_0()
                            .text_color(ShellDeckColors::text_muted()),
                    )
                    .child(t!("user.requests.site_none").to_string())
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.issue_new_site_id = None;
                        this.issue_site_menu = false;
                        cx.notify();
                    })),
            );

            let sites: Vec<ManagedSiteInfo> = self
                .site_directory
                .as_ref()
                .map(|directory| directory.sites.clone())
                .unwrap_or_default();
            let mut matches: Vec<ManagedSiteInfo> = sites
                .into_iter()
                .filter(|site| {
                    query.is_empty()
                        || site.display_label().to_lowercase().contains(&query)
                        || site.host.to_lowercase().contains(&query)
                        || site.tenant_name.to_lowercase().contains(&query)
                })
                .collect();
            matches.sort_by_key(|site| site.display_label().to_lowercase());
            // Capped: 167 sites in one popover is a scroll marathon, and the
            // search field above is the way through. The count says so.
            let total = matches.len();
            for (index, site) in matches.into_iter().take(40).enumerate() {
                let id = site.site_id.clone();
                let selected = selected_id.as_deref() == Some(id.as_str());
                let host = site.host.trim().to_string();
                rows = rows.child(
                    div()
                        .id(("iss-np-site-opt", index))
                        .flex()
                        .items_start()
                        .gap(px(8.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .when(selected, |el| el.bg(ShellDeckColors::selected_bg()))
                        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                        .child(
                            svg()
                                .path(lucide_path("globe"))
                                .size(px(13.0))
                                .flex_shrink_0()
                                .mt(px(2.0))
                                .text_color(ShellDeckColors::text_muted()),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(px(12.0))
                                        .text_color(ShellDeckColors::text_primary())
                                        .child(site.display_label()),
                                )
                                .when(!host.is_empty(), |el| {
                                    el.child(
                                        div()
                                            .truncate()
                                            .text_size(px(10.0))
                                            .text_color(ShellDeckColors::text_muted())
                                            .child(host.clone()),
                                    )
                                }),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.issue_new_site_id = Some(id.clone());
                            this.issue_site_menu = false;
                            cx.notify();
                        })),
                );
            }

            let mut panel = div()
                .id("iss-np-site-menu")
                .w(px(264.0))
                .p(px(4.0))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .bg(ShellDeckColors::bg_surface())
                .border_1()
                .border_color(ShellDeckColors::border())
                .rounded(px(9.0))
                .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                    cx.stop_propagation();
                })
                .child(
                    div().px(px(2.0)).pt(px(2.0)).child(
                        Input::new(&self.issue_site_search)
                            .size(InputSize::Sm)
                            .placeholder(t!("user.requests.site_placeholder").to_string()),
                    ),
                )
                .child(rows);
            if total > 40 {
                panel = panel.child(
                    div()
                        .px(px(8.0))
                        .py(px(5.0))
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("user.requests.site_more", count = total - 40).to_string()),
                );
            }

            // Anchored on the CHIP, not on the pointer. `Local` mode measures
            // from this element's own layout position, so the panel lines up
            // with the chip's left edge no matter where inside it you clicked —
            // anchoring on `event.position()` made it slide sideways.
            // `deferred` keeps it above the frame and out of its clip.
            site_chip = site_chip.child(
                deferred(
                    anchored()
                        .position_mode(gpui::AnchoredPositionMode::Local)
                        // Chip height (24) + 6. The gap is the same 6px for
                        // every picker in this sheet; only the chip height
                        // differs, so only that part of the sum changes.
                        .position(point(gpui::px(0.0), gpui::px(30.0)))
                        .anchor(gpui::Corner::TopLeft)
                        .child(panel),
                )
                .with_priority(3),
            );
        }

        if self.issue_priority_menu {
            let current = self.issue_new_priority.clone();
            let mut list = div()
                .id("iss-np-priority-menu")
                .w(px(168.0))
                .p(px(4.0))
                .flex()
                .flex_col()
                .gap(px(1.0))
                .bg(ShellDeckColors::bg_surface())
                .border_1()
                .border_color(ShellDeckColors::border())
                .rounded(px(9.0))
                .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                    cx.stop_propagation();
                });
            for p in ["low", "normal", "high", "urgent"] {
                let selected = current == p;
                list = list.child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "iss-np-opt-{p}"
                        ))))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(6.0))
                        .cursor_pointer()
                        .when(selected, |el| el.bg(ShellDeckColors::selected_bg()))
                        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                        .child(priority_badge(p))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.issue_new_priority = p.to_string();
                            this.issue_priority_menu = false;
                            cx.notify();
                        })),
                );
            }
            prio_row = prio_row.child(
                deferred(
                    anchored()
                        .position_mode(gpui::AnchoredPositionMode::Local)
                        // Chip height (24) + 6. The gap is the same 6px for
                        // every picker in this sheet; only the chip height
                        // differs, so only that part of the sum changes.
                        .position(point(gpui::px(0.0), gpui::px(30.0)))
                        .anchor(gpui::Corner::TopLeft)
                        .child(list),
                )
                .with_priority(3),
            );
        }

        // Title and body share one frame, like a mail composer: the `Composer`
        // owns the border and the focus ring, so both fields are `Bare`.
        let title_input = Input::new(&self.issue_title_state)
            .variant(InputVariant::Bare)
            // Lg = 16px against the body's 13px: the title must read as the
            // headline of the request, not as another field.
            .size(InputSize::Lg)
            .placeholder(t!("user.requests.title_placeholder").to_string())
            .on_enter({
                let entity = cx.entity();
                move |_value, cx| {
                    entity.update(cx, |ws, cx| ws.submit_new_request(cx));
                }
            });

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
                        // Collapsed, this is a read-only badge: the whole
                        // header row is the collapsible's trigger, so a
                        // clickable picker inside it would fight the toggle for
                        // the same click. It only becomes a picker once the
                        // panel is open — and then it lives in the composer
                        // footer below, never in two places at once.
                        .when(!expanded, |row| {
                            row.child(ai_provider_badge(self.app_config.ai.backend, &model))
                        })
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
                    // The fifth surface on the shared composer. It used to be
                    // the old shape — a bordered `Input` with a labelled button
                    // beside it — sitting directly above the composer it was
                    // supposed to feed, which made the panel read as a
                    // different kind of field.
                    //
                    // Round arrow, not a named button: this sends a prompt, it
                    // does not commit the form. "Créer" below is the commit.
                    Composer::new("user-request-ai-composer", &self.issue_ai_prompt_state)
                        .placeholder(t!("user.requests.ai.placeholder").to_string())
                        .min_rows(2)
                        .max_rows(5)
                        .disabled(self.issue_ai_loading)
                        .commit_enabled(!self.issue_ai_loading)
                        .option(self.render_ai_backend_picker(&model, cx))
                        .on_commit({
                            let entity = cx.entity();
                            move |cx| {
                                entity.update(cx, |this, cx| {
                                    this.generate_new_request_with_ai(cx);
                                });
                            }
                        }),
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
                            cx,
                        ))
                        .child(animated_loading_text(
                            "request-ai-thinking-text",
                            t!("user.requests.ai.generating").to_string(),
                            cx,
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
                        // Collapsing hides the picker; leaving its flag set
                        // would reopen the panel with the menu already down.
                        workspace.issue_ai_backend_menu = false;
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

        // Everything below used to be six stacked rows: a site label + select,
        // a title row with its own AI button, a body box, three rows of
        // attachment controls, and a footer pairing the priority chips with
        // Create. It is one object now — the same `Composer` the assistant
        // uses, with site and priority as context chips and the title in the
        // frame above the body.
        let ai_naming = self.ai_backend_available() && self.app_config.ai.allows(AiSurface::Naming);
        let attachments_open =
            self.issue_attachments_open || !self.issue_new_attachments.is_empty();
        let attachment_count = self.issue_new_attachments.len();

        // `create_issue_now` refuse un titre vide par un `return` silencieux :
        // sans cette condition le bouton restait plein, le clic ne produisait
        // rien du tout, et rien n'expliquait pourquoi. Le formulaire de
        // connexion désactive déjà son action principale de la même façon.
        let title_filled = !self.issue_title_state.read(cx).content().trim().is_empty();

        let mut composer =
            Composer::new("user-new-request-composer", &self.issue_body_state)
                .placeholder(t!("user.requests.body_placeholder").to_string())
                .min_rows(5)
                .max_rows(14)
                .commit_enabled(title_filled && !self.issue_attachment_busy)
                .commit(ComposerCommit::Labeled(
                    t!("user.requests.create").to_string().into(),
                ))
                // Site and priority describe the request, not its wording: they
                // belong in the context row, where the assistant puts its own.
                .context(site_chip)
                .context(prio_row)
                // Title and body inside one frame, separated by a hairline.
                .lead(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().px(px(2.0)).pt(px(4.0)).child(title_input))
                        .child(div().h(px(1.0)).mx(px(12.0)).bg(ShellDeckColors::border())),
                )
                .action(
                    Button::new("iss-attach", "")
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .icon(IconSource::from("plus"))
                        .tooltip(t!("user.requests.attachments.title").to_string())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.issue_attachments_open = !this.issue_attachments_open;
                            cx.notify();
                        })),
                )
                .on_commit({
                    let entity = cx.entity();
                    move |cx| {
                        entity.update(cx, |this, cx| this.submit_new_request(cx));
                    }
                })
                .footnote(div().flex().items_center().gap(px(6.0)).child(
                    if attachment_count == 0 {
                        t!("user.requests.attachments.none").to_string()
                    } else {
                        t!("user.requests.attachments.count", count = attachment_count).to_string()
                    },
                ));
        if ai_naming {
            composer = composer.action(
                Button::new("request-ai-name", "")
                    .variant(ButtonVariant::Ghost)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("sparkles"))
                    .tooltip(t!("ai.naming.action").to_string())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_ai_workflow(
                            AiWorkflowTarget::EntityNaming {
                                kind: AiNamingKind::Issue,
                                target_id: "new-request".to_string(),
                            },
                            cx,
                        );
                    })),
            );
        }

        inner = inner.child(composer);

        // Site picker popover — mounted at sheet level like the priority one,
        // and wider than its chip so the site names are readable.

        // Mounted here, not inside the composer's context row. Nested there it
        // was trapped under the frame's `overflow_hidden`, and the oversized
        // backdrop I had wrapped it in threw off `snap_to_window_with_margin`,
        // which decided there was no room below and flipped the menu above the
        // chip. This is the same shape as the titlebar account popover: a
        // full-panel dismiss layer plus a `deferred(anchored())` menu.
        // Unfolded on demand — or on its own once something is attached, so a
        // pasted image never lands somewhere invisible.
        if attachments_open {
            inner = inner.child(self.render_issue_attachment_picker(
                IssueAttachmentTarget::NewRequest,
                true,
                cx,
            ));
        }

        self.render_user_sheet(
            "user-new-request-sheet",
            t!("user.requests.new").to_string(),
            Some("plus"),
            self.user_new_request_sheet_dismissing,
            is_maximized,
            inner,
            None,
            None,
            |this, cx| this.close_new_request_sheet(cx),
            cx,
        )
    }

    /// The selected-request detail rendered as a right-side sheet.
    pub(super) fn render_user_issue_detail_sheet(
        &self,
        iss: Issue,
        is_maximized: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let inner = self.render_user_issue_detail(&iss, window, cx);
        let footer = self
            .render_user_issue_detail_footer(is_maximized, cx)
            .into_any_element();
        self.render_user_sheet(
            "user-issue-detail-sheet",
            t!("user.requests.detail_title").to_string(),
            Some("tag"),
            self.user_issue_detail_dismissing,
            is_maximized,
            inner,
            Some(footer),
            Some(&self.user_issue_thread_scroll),
            |this, cx| this.close_user_issue_detail(cx),
            cx,
        )
    }

    pub(super) fn render_user_issue_detail(
        &self,
        iss: &Issue,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let font_size = px(12.5).to_pixels(window.rem_size());
        let link_handler: crate::support_view::thread::ThreadLinkHandler = {
            let workspace = cx.entity();
            Rc::new(move |url, window, cx| {
                if let Some(action) =
                    crate::support_view::thread::ThreadLinkAction::new(url, window.mouse_position())
                {
                    workspace.update(cx, |this, cx| {
                        this.issue_thread_link_action = Some(action);
                        cx.notify();
                    });
                }
            })
        };
        let account = self.app_config.account.as_ref();
        let author_is_mine = |author: &str| {
            let author = author.trim().to_ascii_lowercase();
            !author.is_empty()
                && account.is_some_and(|account| {
                    let name = account.name.trim().to_ascii_lowercase();
                    let email = account.email.trim().to_ascii_lowercase();
                    (!name.is_empty() && author == name) || (!email.is_empty() && author == email)
                })
        };
        let source_channel = match iss.source.as_str() {
            "slack" | "github" | "manage" | "email" => Some(SharedString::from(iss.source.clone())),
            _ => None,
        };
        let opening_author = if iss.requested_by.trim().is_empty() {
            t!("support.issue.description").to_string()
        } else {
            iss.requested_by.clone()
        };
        let opening_attachments = (!iss.attachments.is_empty())
            .then(|| self.render_stored_attachments(&iss.attachments, cx));
        let opening = human_message(
            HumanMessageMeta {
                author: opening_author.clone().into(),
                mine: author_is_mine(&opening_author),
                at: iss.created_at,
                channel: source_channel.clone(),
            },
            iss.body.clone(),
            opening_attachments,
            ThreadMessageExtras {
                link_handler: Some(link_handler.clone()),
                ..Default::default()
            },
            font_size,
        );

        // The timeline owns the spacing between messages; attachments stay
        // inside their semantic message instead of becoming orphan rows.
        let mut thread = div()
            .flex()
            .flex_col()
            .gap(px(20.0))
            .mt(px(12.0))
            .child(opening);
        for c in &iss.comments {
            let attachments = (!c.attachments.is_empty())
                .then(|| self.render_stored_attachments(&c.attachments, cx));
            if c.is_note() {
                let kind = match c.kind.as_str() {
                    "status" => ThreadNoteKind::Status,
                    "github" => ThreadNoteKind::Github,
                    "system" if c.body.trim_start().starts_with("Dispatch") => {
                        ThreadNoteKind::Dispatch
                    }
                    _ => ThreadNoteKind::System,
                };
                let actor = (!c.author.trim().is_empty()).then(|| c.author.clone());
                let mut item = div().flex().flex_col().gap(px(6.0)).child(thread_note(
                    c.body.clone(),
                    actor,
                    c.at,
                    kind,
                    px(11.5).to_pixels(window.rem_size()),
                ));
                if let Some(attachments) = attachments {
                    item = item.child(attachments);
                }
                thread = thread.child(item);
            } else {
                let channel = if c.channel.trim().is_empty() {
                    source_channel.clone()
                } else {
                    Some(SharedString::from(c.channel.clone()))
                };
                let author = if c.author.trim().is_empty() {
                    t!("support.issue.comment").to_string()
                } else {
                    c.author.clone()
                };
                thread = thread.child(human_message(
                    HumanMessageMeta {
                        author: author.clone().into(),
                        mine: author_is_mine(&author),
                        at: c.at,
                        channel,
                    },
                    c.body.clone(),
                    attachments,
                    ThreadMessageExtras {
                        link_handler: Some(link_handler.clone()),
                        ..Default::default()
                    },
                    font_size,
                ));
            }
        }

        // Identification, state and destructive actions are three different
        // levels. Keeping all of them on one flex row made the title absorb
        // every bit of compression in the 550 px sheet (U-17).
        let status_color = match iss.status.as_str() {
            "blocked" => ShellDeckColors::error(),
            "done" | "closed" => ShellDeckColors::success(),
            "triaging" => ShellDeckColors::text_muted(),
            _ => ShellDeckColors::primary(),
        };
        let status_label = crate::support_view::issue_status_label(&iss.status);
        let status_label = status_label
            .chars()
            .next()
            .map(|first| {
                first.to_uppercase().collect::<String>() + &status_label[first.len_utf8()..]
            })
            .unwrap_or(status_label);
        let opened_age = crate::i18n::rel_time(iss.created_at);
        let metadata = div()
            .flex()
            .w_full()
            .items_center()
            .flex_wrap()
            .gap(px(6.0))
            .min_w(px(0.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .min_h(px(22.0))
                    .px(px(8.0))
                    .rounded_full()
                    .bg(ShellDeckColors::badge_bg())
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_size(px(10.5))
                    .text_color(ShellDeckColors::text_primary())
                    .child(div().size(px(6.0)).rounded_full().bg(status_color))
                    .child(status_label),
            )
            .children(
                iss.site_label
                    .as_ref()
                    .filter(|label| !label.trim().is_empty())
                    .map(|label| {
                        // The API label may append a technical site alias and
                        // host after an em dash. The customer header names the
                        // site; it does not clip that implementation detail
                        // into a visually meaningless ellipsis (U-17).
                        let display_label = label
                            .split_once(" — ")
                            .map(|(name, _)| name.trim())
                            .unwrap_or_else(|| label.trim());
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.0))
                            .min_h(px(22.0))
                            .px(px(8.0))
                            .rounded_full()
                            .bg(ShellDeckColors::badge_bg())
                            .flex_shrink_0()
                            .whitespace_nowrap()
                            .text_size(px(10.5))
                            .text_color(ShellDeckColors::text_muted())
                            .child(lucide_icon("server", 11.0, ShellDeckColors::text_muted()))
                            .child(display_label.to_string())
                    }),
            )
            .children(iss.github.as_ref().map(|g| {
                div()
                    .id("uiss-gh")
                    .flex_shrink_0()
                    .text_size(px(10.5))
                    .text_color(ShellDeckColors::primary())
                    .cursor_pointer()
                    .child(t!("user.github_issue", number = g.number).to_string())
                    .on_click({
                        let url = g.url.clone();
                        cx.listener(move |_t, _: &ClickEvent, _, _cx| {
                            let _ = cloud_account::open_in_browser(&url);
                        })
                    })
            }))
            .children((!opened_age.is_empty()).then(|| {
                div()
                    .flex_shrink_0()
                    .whitespace_nowrap()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("user.requests.opened", age = opened_age).to_string())
            }));

        let can_delete = self.is_user_visible_issue(iss);
        let title_row = div()
            .flex()
            .w_full()
            .items_start()
            .gap(px(10.0))
            .min_w(px(0.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .text_size(px(15.0))
                    .line_height(relative(1.4))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_primary())
                    .child(crate::external_content::external_title(&iss.title)),
            )
            .when(can_delete, |row| {
                let workspace = cx.entity();
                let issue_id = iss.id.clone();
                row.child(
                    Popover::new("user-issue-detail-actions-popover")
                        .anchor(Corner::TopRight)
                        .trigger(
                            IconButton::new("ellipsis")
                                .variant(ButtonVariant::Outline)
                                .size(gpui::px(28.0))
                                .icon_size(gpui::px(14.0))
                                .rounded_full(),
                        )
                        .content(move |window, cx| {
                            let workspace = workspace.clone();
                            let issue_id = issue_id.clone();
                            cx.new(move |content_cx| {
                                PopoverContent::new(window, content_cx, move |_window, cx| {
                                    let workspace = workspace.clone();
                                    let issue_id = issue_id.clone();
                                    div()
                                        .mx(px(-6.0))
                                        .my(px(-4.0))
                                        .flex()
                                        .flex_col()
                                        .min_w(px(184.0))
                                        .gap(px(2.0))
                                        .child(
                                            div()
                                                .px(px(9.0))
                                                .pt(px(2.0))
                                                .pb(px(3.0))
                                                .text_size(px(9.5))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(ShellDeckColors::text_muted())
                                                .child(
                                                    t!("user.requests.actions")
                                                        .to_string()
                                                        .to_uppercase(),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .id("user-issue-detail-delete-action")
                                                .flex()
                                                .items_center()
                                                .gap(px(8.0))
                                                .px(px(9.0))
                                                .py(px(7.0))
                                                .rounded(px(6.0))
                                                .cursor_pointer()
                                                .text_size(px(11.5))
                                                .text_color(ShellDeckColors::error())
                                                .hover(|style| {
                                                    style.bg(ShellDeckColors::error().opacity(0.10))
                                                })
                                                .child(lucide_icon(
                                                    "trash-2",
                                                    13.0,
                                                    ShellDeckColors::error(),
                                                ))
                                                .child(
                                                    t!("user.requests.delete_action").to_string(),
                                                )
                                                .on_click(cx.listener(
                                                    move |_content, _: &ClickEvent, _, cx| {
                                                        workspace.update(cx, |this, cx| {
                                                            this.confirm_issue_delete =
                                                                Some(issue_id.clone());
                                                            cx.notify();
                                                        });
                                                        cx.emit(DismissEvent);
                                                    },
                                                )),
                                        )
                                        .into_any_element()
                                })
                            })
                        }),
                )
            });

        let heading = div()
            .flex()
            .flex_col()
            .min_w(px(0.0))
            .mx(px(-16.0))
            .px(px(16.0))
            .pb(px(12.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .gap(px(9.0))
            .child(title_row)
            .child(metadata);

        // Detail content flows directly inside the sheet chrome — no inner box
        // (bg / border / rounded) so the sheet reads as a single surface. This
        // direct flex child grows to the viewport when the thread is short and
        // aligns its latest message against the fixed composer. With a long
        // thread it keeps its intrinsic height and lets the tracked parent own
        // the only scroll range (UX-023).
        div()
            .flex()
            .flex_col()
            .flex_grow()
            .flex_shrink_0()
            .justify_end()
            .gap(px(8.0))
            .mt(px(10.0))
            .child(heading)
            .child(thread)
    }

    /// Reply controls stay anchored below the independently scrollable thread.
    /// The optional attachment tools expand upward and are height-capped so a
    /// long draft never pushes the composer outside the sheet.
    pub(super) fn render_user_issue_detail_footer(
        &self,
        is_maximized: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let empty = self
            .issue_comment_state
            .read(cx)
            .content()
            .trim()
            .is_empty();
        let has_attachments = !self.issue_comment_attachments.is_empty();
        let attachments_open = self.issue_comment_attachments_open || has_attachments;
        let send_entity = cx.entity();
        let composer = Composer::new("user-issue-comment-composer", &self.issue_comment_state)
            .placeholder(t!("user.requests.comment_placeholder").to_string())
            .min_rows(1)
            .max_rows(7)
            .commit_enabled(!self.issue_attachment_busy && (!empty || has_attachments))
            .action(
                IconButton::new("plus")
                    .variant(if attachments_open {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    })
                    .size(gpui::px(28.0))
                    .icon_size(gpui::px(14.0))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.issue_comment_attachments_open = !this.issue_comment_attachments_open;
                        cx.notify();
                    })),
            )
            .on_commit(move |cx| {
                send_entity.update(cx, |this, cx| this.submit_issue_comment(cx));
            })
            .footnote(
                div()
                    .flex()
                    .items_center()
                    .gap(px(9.0))
                    .child(t!("ai.assistant.hint.send").to_string())
                    .child(t!("ai.assistant.hint.newline").to_string()),
            );

        let mut footer = div()
            .flex()
            .flex_col()
            .flex_shrink_0()
            .gap(px(8.0))
            .px(px(16.0))
            .pt(px(10.0))
            .pb(px(14.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_surface())
            .on_action(cx.listener(|this, _: &Paste, _, cx| {
                if this.paste_issue_attachment(IssueAttachmentTarget::Comment, cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }));

        if !is_maximized {
            // This opaque footer paints after the already-rounded sheet
            // panel, so it is the actual owner of the window's bottom-right
            // pixels. Repeat the root radius directly on this paint layer.
            footer = footer.rounded_br(use_theme().tokens.radius_xl);
        }

        if attachments_open {
            footer = footer.child(
                div()
                    .id("user-issue-attachment-tools")
                    .max_h(px(220.0))
                    .overflow_y_scroll()
                    .child(self.render_issue_attachment_picker(
                        IssueAttachmentTarget::Comment,
                        false,
                        cx,
                    )),
            );
        }
        footer.child(composer)
    }
}
