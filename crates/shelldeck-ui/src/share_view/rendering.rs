use super::*;
use crate::{icons::lucide_icon, scale::px, theme::ShellDeckColors};
use adabraka_ui::components::{
    button::{Button, ButtonSize, ButtonVariant},
    confirm_dialog::Dialog,
    icon_button::IconButton,
    icon_source::IconSource,
    input::{Input, InputSize},
    progress::{ProgressBar, ProgressSize},
};

impl ShareView {
    fn review_dialog(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let close = cx.entity();
        let mut rows = div()
            .id("share-review-files")
            .debug_selector(|| "share-review-files".into())
            .flex()
            .flex_col()
            .gap(px(4.0))
            .p(px(12.0));
        for (index, draft) in self.drafts.iter().enumerate() {
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .min_w(px(0.0))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(13.0))
                            .child(draft.upload.filename.clone()),
                    )
                    .child(
                        Button::new(
                            SharedString::from(format!("share-review-remove-{index}")),
                            t!("share.remove").to_string(),
                        )
                        .debug_selector(|| format!("share-review-remove-{index}"))
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.busy && index < this.drafts.len() {
                                this.drafts.remove(index);
                            }
                            if this.drafts.is_empty() {
                                this.review_drafts = false;
                            }
                            cx.notify();
                        })),
                    ),
            );
        }
        let height = (window.viewport_size().height - gpui::px(180.0))
            .clamp(gpui::px(80.0), gpui::px(280.0));
        Dialog::new()
            .width(gpui::px(440.0))
            .on_backdrop_click(move |_, cx| {
                close.update(cx, |this, cx| {
                    this.review_drafts = false;
                    cx.notify();
                })
            })
            .header(
                div()
                    .p(px(16.0))
                    .flex()
                    .flex_col()
                    .gap(px(6.0))
                    .child(
                        div()
                            .text_size(px(16.0))
                            .child(t!("share.review", count = self.drafts.len()).to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("share.review_help").to_string()),
                    ),
            )
            .content(
                div().h(height).min_h(px(0.0)).child(
                    adabraka_ui::components::scrollable::Scrollable::vertical(rows)
                        .always_show_scrollbars(),
                ),
            )
            .footer(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .p(px(12.0))
                    .child(
                        Button::new(
                            "share-review-clear",
                            t!("share.clear_selection").to_string(),
                        )
                        .debug_selector(|| "share-review-clear".into())
                        .variant(ButtonVariant::Ghost)
                        .size(ButtonSize::Sm)
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            if !this.busy {
                                this.drafts.clear();
                                this.review_drafts = false;
                                cx.notify();
                            }
                        })),
                    )
                    .child(
                        Button::new("share-review-close", t!("share.done").to_string())
                            .debug_selector(|| "share-review-close".into())
                            .size(ButtonSize::Sm)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.review_drafts = false;
                                cx.notify();
                            })),
                    ),
            )
            .into_any_element()
    }
    fn progress_footer(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self
            .upload_progress
            .as_ref()
            .expect("active upload progress")
            .read();
        let current = state.current.min(self.drafts.len().saturating_sub(1));
        let name = self
            .drafts
            .get(current)
            .map(|draft| draft.upload.filename.clone())
            .unwrap_or_default();
        let stopping = self.cancelled.load(Ordering::Acquire);
        let label = if stopping {
            t!("share.stopping").to_string()
        } else {
            t!(
                "share.transfer",
                file = name,
                index = current + 1,
                count = self.drafts.len()
            )
            .to_string()
        };
        let bar = if state.total == 0 || state.done == 0 {
            ProgressBar::indeterminate()
        } else {
            ProgressBar::new(state.done as f32 / state.total as f32)
        };
        let mut status = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w(px(0.0))
            .gap(px(4.0))
            .child(
                div()
                    .debug_selector(|| "share-transfer-label".into())
                    .text_size(px(12.0))
                    .truncate()
                    .child(label),
            )
            .child(bar.size(ProgressSize::Sm));
        if state.total > 0 {
            status = status.child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!(
                        "{} / {}",
                        human_bytes(state.done),
                        human_bytes(state.total)
                    )),
            );
        }
        div()
            .debug_selector(|| "share-transfer-progress".into())
            .flex()
            .items_center()
            .gap(px(12.0))
            .flex_shrink_0()
            .child(status)
            .child(
                Button::new("share-stop", t!("share.stop").to_string())
                    .debug_selector(|| "share-stop".into())
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Outline)
                    .disabled(stopping)
                    .on_click(cx.listener(|this, _, _, cx| this.stop_upload(cx))),
            )
            .into_any_element()
    }

    fn render_file(
        &self,
        file: &shelldeck_core::config::share::ShareFile,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let viewer = file.viewer_url.clone();
        let copy = viewer.clone();
        let id = file.id.clone();
        let expiry = file
            .expires_at
            .as_deref()
            .and_then(|date| chrono::DateTime::parse_from_rfc3339(date).ok())
            .map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%d/%m/%Y %H:%M")
                    .to_string()
            })
            .unwrap_or_else(|| t!("share.forever").to_string());
        div()
            .h(px(82.0))
            .py(px(4.0))
            .child(
                div()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(12.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(lucide_icon(
                        if file.kind == "image" {
                            "paperclip"
                        } else {
                            "file-text"
                        },
                        20.0,
                        ShellDeckColors::text_muted(),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.0))
                                    .child(file.filename.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(format!("{} · {}", human_bytes(file.bytes), expiry)),
                            ),
                    )
                    .child(
                        Button::new(
                            SharedString::from(format!("share-copy-{}", file.id)),
                            t!("share.copy").to_string(),
                        )
                        .debug_selector(|| format!("share-copy-{}", file.id))
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .on_click(move |_, _, cx| {
                            if share_url(&copy) {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                            }
                        }),
                    )
                    .child(
                        Button::new(
                            SharedString::from(format!("share-open-{}", file.id)),
                            t!("share.open").to_string(),
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .on_click(move |_, _, _| {
                            if share_url(&viewer) {
                                let _ = open::that_detached(viewer.clone());
                            }
                        }),
                    )
                    .child({
                        let rename_id = file.id.clone();
                        let original = file.filename.clone();
                        div()
                            .id(SharedString::from(format!("share-rename-{}", file.id)))
                            .debug_selector(|| format!("share-rename-{}", file.id))
                            .child(
                                IconButton::new("pencil")
                                    .variant(ButtonVariant::Ghost)
                                    .size(gpui::px(28.0))
                                    .icon_size(gpui::px(13.0))
                                    .disabled(self.busy)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.begin_rename(
                                            rename_id.clone(),
                                            original.clone(),
                                            window,
                                            cx,
                                        );
                                    })),
                            )
                    })
                    .child(
                        Button::new(
                            SharedString::from(format!("share-delete-{}", file.id)),
                            t!("share.delete").to_string(),
                        )
                        .debug_selector(|| format!("share-delete-{}", file.id))
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.confirm_delete = Some(id.clone());
                            cx.notify();
                        })),
                    ),
            )
            .into_any_element()
    }
    fn rename_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(draft) = self.rename.as_ref() else {
            return div().into_any_element();
        };
        let cancel = cx.entity();
        let enter = cancel.clone();
        let confirm = cancel.clone();
        Dialog::new()
            .width(gpui::px(440.0))
            .on_backdrop_click({
                let cancel = cancel.clone();
                move |_, cx| {
                    cancel.update(cx, |this, cx| {
                        this.rename = None;
                        cx.notify();
                    })
                }
            })
            .header(
                div()
                    .p(px(16.0))
                    .text_size(px(16.0))
                    .child(t!("share.rename_title").to_string()),
            )
            .content(
                div()
                    .p(px(16.0))
                    .flex()
                    .flex_col()
                    .gap(px(8.0))
                    .child(
                        div().debug_selector(|| "share-rename-input".into()).child(
                            Input::new(&draft.input)
                                .size(InputSize::Sm)
                                .on_enter(move |_, cx| {
                                    enter.update(cx, |this, cx| this.rename_confirmed(cx))
                                }),
                        ),
                    )
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("share.rename_help").to_string()),
                    )
                    .children(self.error.clone().map(|error| {
                        div()
                            .debug_selector(|| "share-rename-error".into())
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::error())
                            .child(error)
                    })),
            )
            .footer(
                div()
                    .p(px(16.0))
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        Button::new("share-rename-cancel", t!("share.cancel").to_string())
                            .debug_selector(|| "share-rename-cancel".into())
                            .variant(ButtonVariant::Outline)
                            .on_click(move |_, _, cx| {
                                cancel.update(cx, |this, cx| {
                                    this.rename = None;
                                    cx.notify();
                                })
                            }),
                    )
                    .child(
                        Button::new("share-rename-confirm", t!("share.rename_save").to_string())
                            .debug_selector(|| "share-rename-confirm".into())
                            .on_click(move |_, _, cx| {
                                confirm.update(cx, |this, cx| this.rename_confirmed(cx))
                            }),
                    ),
            )
            .into_any_element()
    }
    fn delete_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let cancel = cx.entity();
        let confirm = cancel.clone();
        Dialog::new()
            .width(gpui::px(420.0))
            .on_backdrop_click({
                let cancel = cancel.clone();
                move |_, cx| {
                    cancel.update(cx, |this, cx| {
                        this.confirm_delete = None;
                        cx.notify();
                    })
                }
            })
            .header(
                div()
                    .p(px(16.0))
                    .text_size(px(16.0))
                    .child(t!("share.delete_title").to_string()),
            )
            .content(
                div()
                    .p(px(16.0))
                    .text_size(px(13.0))
                    .child(t!("share.delete_body").to_string()),
            )
            .footer(
                div()
                    .p(px(16.0))
                    .flex()
                    .justify_end()
                    .gap(px(8.0))
                    .child(
                        Button::new("share-delete-cancel", t!("share.cancel").to_string())
                            .debug_selector(|| "share-delete-cancel".into())
                            .variant(ButtonVariant::Outline)
                            .on_click(move |_, _, cx| {
                                cancel.update(cx, |this, cx| {
                                    this.confirm_delete = None;
                                    cx.notify();
                                })
                            }),
                    )
                    .child(
                        Button::new("share-delete-confirm", t!("share.delete").to_string())
                            .on_click(move |_, _, cx| {
                                confirm.update(cx, |this, cx| this.delete_confirmed(cx))
                            }),
                    ),
            )
            .into_any_element()
    }
}
impl Render for ShareView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(annotator) = &self.annotator {
            return div()
                .size_full()
                .child(annotator.clone())
                .into_any_element();
        }
        let mut root = div()
            .id("personal-share")
            .size_full()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .gap(px(12.0))
            .p(px(16.0))
            .bg(ShellDeckColors::bg_primary())
            .text_color(ShellDeckColors::text_primary())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && this.rename.is_some() {
                    this.rename = None;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                if event.keystroke.key == "escape" && this.review_drafts {
                    this.review_drafts = false;
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                let modifiers = event.keystroke.modifiers;
                if this.rename.is_none()
                    && !this.search.read(cx).focus_handle(cx).is_focused(window)
                    && event.keystroke.key.eq_ignore_ascii_case("v")
                    && (modifiers.control || modifiers.platform)
                {
                    this.paste(cx);
                    cx.stop_propagation();
                }
            }))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                this.add_paths(paths.paths().to_vec(), cx)
            }));
        let mut controls = div()
            .id("share-intake")
            .debug_selector(|| "share-intake".into())
            .flex()
            .flex_col()
            .gap(px(12.0))
            .max_h(relative(0.5))
            .flex_shrink_0()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.0))
                    .flex_wrap()
                    .child(
                        div()
                            .text_size(px(23.0))
                            .font_weight(FontWeight::BOLD)
                            .child("Inklura Share"),
                    )
                    .child(
                        Button::new("share-refresh", t!("share.refresh").to_string())
                            .variant(ButtonVariant::Outline)
                            .size(ButtonSize::Sm)
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    ),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("share.intro").to_string()),
            );
        if let Some(snapshot) = &self.snapshot {
            controls = controls.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(10.0))
                    .text_size(px(12.0))
                    .child(format!(
                        "{} · {} · {} / {}",
                        snapshot.owner.email,
                        snapshot.plan.label,
                        human_bytes(snapshot.usage.bytes),
                        human_bytes(snapshot.plan.storage_bytes)
                    )),
            );
            if snapshot.plan.pending_france || !snapshot.plan.storage_ready {
                controls = controls.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::warning())
                        .child(t!("share.error.france").to_string()),
                );
            }
        }
        if let Some(error) = &self.error {
            controls = controls.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::error())
                    .child(error.clone()),
            );
        }
        if let Some(notice) = &self.notice {
            controls = controls.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::success())
                    .child(notice.clone()),
            );
        }
        controls = controls.child(
            div()
                .flex()
                .gap(px(8.0))
                .flex_wrap()
                .child(
                    Button::new("share-pick", t!("share.files").to_string())
                        .icon(IconSource::from("upload"))
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.choose_files(cx))),
                )
                .child(
                    Button::new("share-capture", t!("share.capture").to_string())
                        .icon(IconSource::from("scan"))
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.capture(cx))),
                )
                .child(
                    Button::new("share-paste", t!("share.paste").to_string())
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| this.paste(cx))),
                )
                .child(
                    Button::new("share-pdf", "Inklura PDF")
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .on_click(cx.listener(|this, _, _, cx| this.open_pdf(cx))),
                ),
        );
        if let Some(snapshot) = &self.snapshot {
            let mut choices = div().flex().items_center().flex_wrap().gap(px(6.0)).child(
                div()
                    .text_size(px(12.0))
                    .child(t!("share.retention").to_string()),
            );
            for days in &snapshot.plan.retention_days {
                let seconds = days * 86400;
                let label = if *days == 0 {
                    t!("share.forever").to_string()
                } else {
                    t!("share.days", count = *days).to_string()
                };
                choices = choices.child(
                    Button::new(SharedString::from(format!("share-retention-{days}")), label)
                        .size(ButtonSize::Sm)
                        .variant(if seconds == self.retention {
                            ButtonVariant::Default
                        } else {
                            ButtonVariant::Outline
                        })
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.retention = seconds;
                            cx.notify();
                        })),
                );
            }
            choices = choices.child(
                Button::new("share-default", t!("share.save_default").to_string())
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| this.save_default(cx))),
            );
            controls = controls.child(choices);
        }
        if !self.drafts.is_empty() {
            let mut drafts = div()
                .id("share-drafts")
                .debug_selector(|| "share-drafts".into())
                .flex()
                .flex_wrap()
                .gap(px(8.0))
                .max_h(px(100.0))
                .min_h(px(0.0))
                .flex_shrink_0()
                .overflow_y_scroll();
            for (index, draft) in self.drafts.iter().enumerate() {
                let mut row = div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .max_w(px(320.0))
                    .min_w(px(0.0));
                if let Some(preview) = &draft.preview {
                    row = row.child(
                        img(preview.image.clone())
                            .w(px(44.0))
                            .h(px(36.0))
                            .object_fit(ObjectFit::Contain),
                    );
                }
                drafts = drafts.child(
                    row.child(
                        div()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(12.0))
                            .child(draft.upload.filename.clone()),
                    )
                    .child(
                        Button::new(
                            SharedString::from(format!("share-draft-remove-{index}")),
                            "×",
                        )
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Ghost)
                        .disabled(self.busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if index < this.drafts.len() {
                                this.drafts.remove(index);
                            }
                            cx.notify();
                        })),
                    ),
                );
            }
            controls = controls.child(drafts);
        }
        if self.busy {
            controls = controls.child(
                div()
                    .text_size(px(12.0))
                    .child(t!("share.working").to_string()),
            );
        }
        root = root.child(controls);
        if self.upload_progress.is_some() {
            root = root.child(self.progress_footer(cx));
        } else if !self.drafts.is_empty() {
            root = root.child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .flex_shrink_0()
                    .child(
                        Button::new(
                            "share-review",
                            t!("share.review", count = self.drafts.len()).to_string(),
                        )
                        .debug_selector(|| "share-review".into())
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Outline)
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.review_drafts = true;
                            cx.notify();
                        })),
                    )
                    .child(
                        Button::new(
                            "share-send",
                            t!("share.send", count = self.drafts.len()).to_string(),
                        )
                        .flex_1()
                        .debug_selector(|| "share-send".into())
                        .disabled(
                            self.busy
                                || self
                                    .snapshot
                                    .as_ref()
                                    .is_none_or(|snapshot| !snapshot.plan.storage_ready),
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
                    ),
            );
        }
        let search = self.search.clone();
        let view = cx.entity();
        root = root.child(
            Input::new(&search)
                .size(InputSize::Sm)
                .placeholder(t!("share.search").to_string())
                .on_change(move |_, cx| view.update(cx, |_, cx| cx.notify())),
        );
        let query = self.search.read(cx).content().to_lowercase();
        let files = self
            .snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .files
                    .iter()
                    .filter(|file| file.filename.to_lowercase().contains(&query))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if files.is_empty() && !self.busy {
            root = root.child(
                div()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("share.empty").to_string()),
            );
        }
        let files = Arc::new(files);
        root = root.child(
            uniform_list(
                "personal-share-files",
                files.len(),
                cx.processor(move |this, range: std::ops::Range<usize>, _, cx| {
                    range
                        .filter_map(|index| files.get(index))
                        .map(|file| this.render_file(file, cx))
                        .collect::<Vec<_>>()
                }),
            )
            .flex_1()
            .min_h(px(0.0)),
        );
        if let Some(snapshot) = &self.snapshot {
            if snapshot.usage.files > snapshot.list_limit as u64 {
                root = root.child(
                    div()
                        .text_size(px(11.0))
                        .child(t!("share.list_limit", count = snapshot.list_limit).to_string()),
                );
            }
        }
        if self.review_drafts {
            root = root.child(self.review_dialog(window, cx));
        }
        if self.confirm_delete.is_some() {
            root = root.child(self.delete_dialog(cx));
        }
        if self.rename.is_some() {
            root = root.child(self.rename_dialog(cx));
        }
        root.into_any_element()
    }
}
pub(crate) fn human_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!(
            "{:.1} {}",
            bytes as f64 / (1024.0 * 1024.0 * 1024.0),
            t!("share.gib")
        )
    } else if bytes >= 1024 * 1024 {
        format!(
            "{:.1} {}",
            bytes as f64 / (1024.0 * 1024.0),
            t!("share.mib")
        )
    } else {
        format!("{} {}", bytes.div_ceil(1024), t!("share.kib"))
    }
}
