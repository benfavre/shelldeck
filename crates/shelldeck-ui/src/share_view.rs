//! Inklura Desktop's personal sharing surface. Capture/annotation reuse the
//! request tools; uploads, gallery and retention use personal Share policies.
use crate::{
    attachment_annotator::AttachmentAnnotator,
    icons::lucide_icon,
    issue_attachments::{capture_region, draft_from_clipboard_image, AttachmentDraft},
    scale::px,
    t,
    theme::ShellDeckColors,
};
use adabraka_ui::components::button::{Button, ButtonSize, ButtonVariant};
use adabraka_ui::components::confirm_dialog::Dialog;
use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input::{Input, InputSize, InputState};
use gpui::{prelude::*, *};
use shelldeck_core::config::share::{
    share_url, ShareClient, ShareError, ShareResult, ShareSnapshot, ShareSource, ShareUpload,
    SharedUpload,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

#[derive(Clone)]
pub enum ShareViewEvent {
    AuthenticationRejected,
}
impl EventEmitter<ShareViewEvent> for ShareView {}
#[derive(Clone)]
struct Draft {
    upload: ShareUpload,
    preview: Option<AttachmentDraft>,
}
struct Outcome {
    snapshot: Option<ShareSnapshot>,
    uploads: Vec<SharedUpload>,
    completed: usize,
    warning: Option<ShareError>,
}
impl Outcome {
    fn snapshot(snapshot: ShareSnapshot) -> Self {
        Self {
            snapshot: Some(snapshot),
            uploads: vec![],
            completed: 0,
            warning: None,
        }
    }
}
pub struct ShareView {
    manage: String,
    device: String,
    cancelled: Arc<AtomicBool>,
    generation: u64,
    snapshot: Option<ShareSnapshot>,
    drafts: Vec<Draft>,
    retention: u64,
    busy: bool,
    error: Option<String>,
    notice: Option<String>,
    confirm_delete: Option<String>,
    annotator: Option<Entity<AttachmentAnnotator>>,
    search: Entity<InputState>,
    focus: FocusHandle,
}
impl ShareView {
    /// Isolated visual fixtures have no device credential and cannot upload.
    #[cfg(debug_assertions)]
    pub fn preview(snapshot: ShareSnapshot, paths: Vec<PathBuf>, cx: &mut Context<Self>) -> Self {
        let mut view = Self::new(cx);
        view.retention = snapshot.default_retention_seconds;
        view.snapshot = Some(snapshot);
        view.drafts = paths
            .into_iter()
            .filter_map(|path| ShareUpload::from_path(path).ok())
            .map(|upload| Draft {
                upload,
                preview: None,
            })
            .collect();
        view
    }
    pub fn focus_in(&self, window: &mut Window) {
        window.focus(&self.focus);
    }
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            manage: String::new(),
            device: String::new(),
            cancelled: Arc::new(AtomicBool::new(false)),
            generation: 0,
            snapshot: None,
            drafts: vec![],
            retention: 7 * 86400,
            busy: false,
            error: None,
            notice: None,
            confirm_delete: None,
            annotator: None,
            search: cx.new(InputState::new),
            focus: cx.focus_handle(),
        }
    }
    pub fn configure(&mut self, manage: String, device: String, cx: &mut Context<Self>) {
        if self.manage == manage && self.device == device {
            return;
        }
        self.clear(cx);
        self.manage = manage;
        self.device = device;
    }
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.cancelled.store(true, Ordering::Release);
        self.cancelled = Arc::new(AtomicBool::new(false));
        self.generation = self.generation.wrapping_add(1);
        self.device.clear();
        self.manage.clear();
        self.snapshot = None;
        self.drafts.clear();
        self.annotator = None;
        self.confirm_delete = None;
        self.error = None;
        self.notice = None;
        self.busy = false;
        self.retention = 7 * 86400;
        self.search = cx.new(InputState::new);
        cx.notify();
    }
    fn run(
        &mut self,
        job: impl FnOnce(ShareClient) -> ShareResult<Outcome> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.busy || self.device.is_empty() {
            return;
        }
        self.busy = true;
        self.error = None;
        self.notice = None;
        let generation = self.generation;
        let manage = self.manage.clone();
        let device = self.device.clone();
        let cancelled = self.cancelled.clone();
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async move { ShareClient::new(&manage, &device, cancelled).and_then(job) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(outcome) => {
                        if let Some(snapshot) = outcome.snapshot {
                            if this.snapshot.is_none()
                                || !snapshot
                                    .plan
                                    .retention_days
                                    .contains(&(this.retention / 86400))
                            {
                                this.retention = snapshot.default_retention_seconds;
                            }
                            this.snapshot = Some(snapshot);
                        }
                        let completed = outcome.completed.min(this.drafts.len());
                        this.drafts.drain(..completed);
                        if let Some(upload) = outcome.uploads.last() {
                            cx.write_to_clipboard(ClipboardItem::new_string(upload.viewer.clone()));
                            this.notice = Some(
                                t!("share.uploaded", count = outcome.uploads.len()).to_string(),
                            );
                        }
                        if let Some(error) = outcome.warning {
                            this.fail(error, cx);
                        }
                    }
                    Err(error) => this.fail(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn fail(&mut self, error: ShareError, cx: &mut Context<Self>) {
        if matches!(error, ShareError::Unauthorized) {
            cx.emit(ShareViewEvent::AuthenticationRejected);
        }
        let key = match &error {
            ShareError::Unauthorized => "share.error.auth",
            ShareError::Cancelled => "share.error.cancelled",
            ShareError::Remote(code) if code == "france_storage_pending" => "share.error.france",
            ShareError::Remote(code) if code == "storage_limit" || code == "file_count_limit" => {
                "share.error.quota"
            }
            ShareError::Invalid("unsupported_type") => "share.error.type",
            ShareError::Remote(code) if code == "unsupported_type" => "share.error.type",
            ShareError::Invalid("too_large") => "share.error.size",
            ShareError::Remote(code) if code == "too_large" => "share.error.size",
            ShareError::Invalid("invalid_retention") | ShareError::Remote(_) => {
                "share.error.policy"
            }
            _ => "share.error.network",
        };
        self.error = Some(t!(key).to_string());
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.run(|client| client.snapshot().map(Outcome::snapshot), cx);
    }
    fn send(&mut self, cx: &mut Context<Self>) {
        if self.drafts.is_empty() || self.snapshot.as_ref().is_none_or(|s| !s.plan.storage_ready) {
            return;
        }
        let drafts = self.drafts.clone();
        let retention = self.retention;
        self.run(
            move |client| {
                let mut uploads = Vec::new();
                let mut warning = None;
                for draft in drafts {
                    match client.upload(&draft.upload, retention, |_, _| {}) {
                        Ok(upload) => uploads.push(upload),
                        Err(error) => {
                            warning = Some(error);
                            break;
                        }
                    }
                }
                let snapshot = client.snapshot().ok();
                Ok(Outcome {
                    completed: uploads.len(),
                    uploads,
                    snapshot,
                    warning,
                })
            },
            cx,
        );
    }
    fn add_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if paths.len() + self.drafts.len() > 20 {
            self.error = Some(t!("share.error.batch").to_string());
            cx.notify();
            return;
        }
        let generation = self.generation;
        self.busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let drafts: ShareResult<Vec<Draft>> = cx
                .background_executor()
                .spawn(async move {
                    paths
                        .into_iter()
                        .map(|path| {
                            ShareUpload::from_path(path).map(|upload| Draft {
                                upload,
                                preview: None,
                            })
                        })
                        .collect()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match drafts {
                    Ok(drafts) => this.drafts.extend(drafts),
                    Err(error) => this.fail(error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn choose_files(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let generation = self.generation;
        let picker = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t!("share.files").to_string().into()),
            starting_directory: None,
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = picker.await {
                let _ = this.update(cx, |this, cx| {
                    if this.generation == generation {
                        this.add_paths(paths, cx);
                    }
                });
            }
        })
        .detach();
    }
    fn add_capture(&mut self, draft: AttachmentDraft, cx: &mut Context<Self>) {
        if self.drafts.len() >= 20 {
            self.error = Some(t!("share.error.batch").to_string());
        } else {
            self.drafts.push(Draft {
                upload: ShareUpload {
                    filename: draft.filename.clone(),
                    content_type: draft.content_type.clone(),
                    source: ShareSource::Bytes(draft.bytes.clone()),
                },
                preview: Some(draft),
            });
        }
        cx.notify();
    }
    fn paste(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let image = cx.read_from_clipboard().and_then(|item| {
            item.entries().iter().find_map(|entry| match entry {
                ClipboardEntry::Image(image) => Some(image.clone()),
                _ => None,
            })
        });
        match image.as_ref().map(draft_from_clipboard_image) {
            Some(Ok(draft)) => self.add_capture(draft, cx),
            Some(Err(error)) => {
                self.error = Some(error);
                cx.notify();
            }
            None => {
                self.error = Some(t!("share.error.clipboard").to_string());
                cx.notify();
            }
        }
    }
    fn capture(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        let generation = self.generation;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { capture_region() })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.busy = false;
                match result {
                    Ok(draft) => this.annotate(draft, cx),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn annotate(&mut self, draft: AttachmentDraft, cx: &mut Context<Self>) {
        let cancel = cx.entity().downgrade();
        let apply = cancel.clone();
        self.annotator = Some(cx.new(|cx| {
            AttachmentAnnotator::new(
                draft,
                move |cx| {
                    if let Some(view) = cancel.upgrade() {
                        view.update(cx, |view, cx| {
                            view.annotator = None;
                            cx.notify();
                        });
                    }
                },
                move |draft, cx| {
                    if let Some(view) = apply.upgrade() {
                        view.update(cx, |view, cx| {
                            view.annotator = None;
                            view.add_capture(draft, cx);
                        });
                    }
                },
                cx,
            )
        }));
        cx.notify();
    }
    fn save_default(&mut self, cx: &mut Context<Self>) {
        let seconds = self.retention;
        self.run(
            move |client| {
                client.set_retention(seconds)?;
                client.snapshot().map(Outcome::snapshot)
            },
            cx,
        );
    }
    pub fn open_pdf(&mut self, cx: &mut Context<Self>) {
        let generation = self.generation;
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { shelldeck_core::config::desktop_pdf::open_pdf_editor() })
                .await;
            let _ = this.update(cx, |this, cx| {
                if generation != this.generation {
                    return;
                }
                let key = match result {
                    Ok(shelldeck_core::config::desktop_pdf::PdfDestination::Application) => {
                        "share.pdf_opened"
                    }
                    Ok(shelldeck_core::config::desktop_pdf::PdfDestination::Downloads) => {
                        "share.pdf_downloads"
                    }
                    Err(_) => "share.pdf_failed",
                };
                this.notice = Some(t!(key).to_string());
                cx.notify();
            });
        })
        .detach();
    }
    fn delete_confirmed(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.confirm_delete.take() else {
            return;
        };
        self.run(
            move |client| {
                client.delete(&id)?;
                client.snapshot().map(Outcome::snapshot)
            },
            cx,
        );
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
impl Focusable for ShareView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for ShareView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                let modifiers = event.keystroke.modifiers;
                if !this.search.read(cx).focus_handle(cx).is_focused(window)
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
        if !self.drafts.is_empty() {
            root = root.child(
                Button::new(
                    "share-send",
                    t!("share.send", count = self.drafts.len()).to_string(),
                )
                .debug_selector(|| "share-send".into())
                .disabled(
                    self.busy
                        || self
                            .snapshot
                            .as_ref()
                            .is_none_or(|snapshot| !snapshot.plan.storage_ready),
                )
                .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
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
        if self.confirm_delete.is_some() {
            root = root.child(self.delete_dialog(cx));
        }
        root.into_any_element()
    }
}
fn human_bytes(bytes: u64) -> String {
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
#[cfg(test)]
mod tests;
