use super::*;
impl AiAssistantView {
    pub(super) fn render_overlay_panels(
        &mut self,
        mut root: Div,
        history_column_visible: bool,
        cx: &mut Context<Self>,
    ) -> Div {
        if self.account_menu_open {
            if let Some(label) = self.account_label.clone() {
                let detail = self.account_detail.clone();
                let panel = div()
                    .id("ai-account-panel")
                    // Anchored to the rail's bottom-right corner, above the
                    // avatar that opened it.
                    .absolute()
                    .bottom(px(52.0))
                    .right(px(62.0))
                    .w(px(226.0))
                    .p(px(4.0))
                    .flex()
                    .flex_col()
                    .gap(px(1.0))
                    .bg(ShellDeckColors::bg_surface())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .rounded(px(10.0))
                    .shadow(
                        vec![BoxShadow {
                            color: hsla(0.0, 0.0, 0.0, 0.45),
                            offset: point(gpui::px(0.0), gpui::px(4.0)),
                            blur_radius: gpui::px(20.0),
                            spread_radius: gpui::px(0.0),
                            inset: false,
                        }]
                        .into(),
                    )
                    .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                        cx.stop_propagation();
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .px(px(9.0))
                            .py(px(8.0))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.5))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(label),
                            )
                            .children(detail.map(|detail| {
                                div()
                                    .truncate()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(detail)
                            })),
                    )
                    .child(
                        div()
                            .h(px(1.0))
                            .mx(px(6.0))
                            .my(px(3.0))
                            .bg(ShellDeckColors::border()),
                    )
                    .child(
                        div()
                            .id("ai-account-settings")
                            .flex()
                            .items_center()
                            .gap(px(8.0))
                            .px(px(9.0))
                            .py(px(7.0))
                            .rounded(px(7.0))
                            .cursor_pointer()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_primary())
                            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                            .child(lucide_icon("settings", 13.0, ShellDeckColors::text_muted()))
                            .child(t!("settings.title").to_string())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.account_menu_open = false;
                                cx.emit(AiAssistantEvent::OpenSettings);
                            })),
                    );
                root = root.child(
                    div()
                        .id("ai-account-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .bottom_0()
                        .occlude()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _e, _window, cx| {
                                this.account_menu_open = false;
                                cx.notify();
                            }),
                        )
                        .child(panel),
                );
            }
        }

        if self.backend_menu_open {
            let current = self.backend;
            let mut menu = div()
                .id("ai-backend-menu")
                .absolute()
                .bottom(px(78.0))
                .right(px(16.0))
                .w(px(208.0))
                .p(px(4.0))
                .flex()
                .flex_col()
                .gap(px(1.0))
                .bg(ShellDeckColors::bg_surface())
                .border_1()
                .border_color(ShellDeckColors::border())
                .rounded(px(10.0))
                .shadow(
                    vec![BoxShadow {
                        color: hsla(0.0, 0.0, 0.0, 0.45),
                        // BoxShadow fields are typed `Pixels` — never rems.
                        offset: point(gpui::px(0.0), gpui::px(4.0)),
                        blur_radius: gpui::px(20.0),
                        spread_radius: gpui::px(0.0),
                        inset: false,
                    }]
                    .into(),
                )
                .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut App| {
                    cx.stop_propagation();
                });
            for (index, (backend, label)) in Self::backend_choices().into_iter().enumerate() {
                let selected = backend == current;
                menu = menu.child(
                    div()
                        .id(("ai-backend-choice", index))
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .px(px(9.0))
                        .py(px(7.0))
                        .rounded(px(7.0))
                        .cursor_pointer()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_primary())
                        .when(selected, |el| el.bg(ShellDeckColors::selected_bg()))
                        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                        .child(ai_provider_icon(
                            backend,
                            14.0,
                            ShellDeckColors::text_primary(),
                        ))
                        .child(div().flex_1().min_w(px(0.0)).truncate().child(label))
                        .when(selected, |el| {
                            el.child(lucide_icon("check", 13.0, ShellDeckColors::primary()))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.backend_menu_open = false;
                            cx.emit(AiAssistantEvent::SelectBackend(backend));
                            cx.notify();
                        })),
                );
            }
            root = root.child(
                // Dismiss layer: a click anywhere else closes the menu.
                div()
                    .id("ai-backend-menu-backdrop")
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _e, _window, cx| {
                            this.backend_menu_open = false;
                            cx.notify();
                        }),
                    )
                    .child(menu),
            );
        }

        if let Some(menu) = self.render_attach_menu(history_column_visible, cx) {
            root = root.child(menu);
        }
        if let Some(picker) = self.render_mention_picker(history_column_visible, cx) {
            root = root.child(picker);
        }

        // Last children: the annotator and the image viewer each own the whole
        // surface while open.
        if let Some(annotator) = self.render_capture_annotator() {
            root = root.child(annotator);
        }
        if let Some(lightbox) = self.render_attachment_lightbox() {
            root = root.child(lightbox);
        }

        if let Some(delete_id) = self.pending_delete {
            root = root.child(
                UiDialog::new()
                    .width(gpui::px(420.0))
                    .header(
                        div()
                            .px(px(16.0))
                            .py(px(14.0))
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("ai.history.delete_title").to_string()),
                    )
                    .content(
                        div()
                            .px(px(16.0))
                            .pb(px(16.0))
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("ai.history.delete_description").to_string()),
                    )
                    .footer(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.0))
                            .p(px(12.0))
                            .border_t_1()
                            .border_color(ShellDeckColors::border())
                            .child(
                                Button::new("ai-delete-cancel", t!("scripts.cancel").to_string())
                                    .variant(ButtonVariant::Ghost)
                                    .size(ButtonSize::Sm)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.pending_delete = None;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new(
                                    "ai-delete-confirm",
                                    t!("ai.history.delete").to_string(),
                                )
                                .variant(ButtonVariant::Destructive)
                                .size(ButtonSize::Sm)
                                .icon(IconSource::from("trash-2"))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.delete_conversation(delete_id, cx);
                                    },
                                )),
                            ),
                    )
                    .on_backdrop_click({
                        let entity = cx.entity();
                        move |_, cx| {
                            entity.update(cx, |this, cx| {
                                this.pending_delete = None;
                                cx.notify();
                            });
                        }
                    }),
            );
        }

        if let Some(action) = self.markdown_link_action.clone() {
            let parent = cx.entity();
            root = root.child(markdown_link_popover(action, move |cx| {
                parent.update(cx, |this, cx| {
                    this.markdown_link_action = None;
                    cx.notify();
                });
            }));
        }

        root
    }
}
