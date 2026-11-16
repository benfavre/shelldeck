use super::*;
use crate::{scale::px, theme::ShellDeckColors};
use adabraka_ui::components::{
    button::{Button, ButtonSize, ButtonVariant},
    confirm_dialog::Dialog,
    input::{Input, InputSize},
    progress::{ProgressBar, ProgressSize},
};

impl ShareView {
    pub(super) fn review_dialog(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
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
    pub(super) fn progress_footer(&self, cx: &mut Context<Self>) -> AnyElement {
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

    pub(super) fn rename_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
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
    pub(super) fn delete_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
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
