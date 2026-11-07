//! Inklura Desktop's personal sharing surface. Capture/annotation reuse the
//! request tools; uploads, gallery and retention use personal Share policies.
use crate::{
    attachment_annotator::AttachmentAnnotator,
    issue_attachments::{capture_region, draft_from_clipboard_image, AttachmentDraft},
    t,
};
use adabraka_ui::components::input::InputState;
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

/// Runs once an external capture request finishes (shows the main window).
pub type CaptureReady = Box<dyn FnOnce(&mut App)>;
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
    upload_progress: Option<Arc<UploadProgress>>,
    review_drafts: bool,
    error: Option<String>,
    notice: Option<String>,
    confirm_delete: Option<String>,
    annotator: Option<Entity<AttachmentAnnotator>>,
    search: Entity<InputState>,
    focus: FocusHandle,
}
impl ShareView {
    /// Native preview examples can exercise overlays without an account.
    #[cfg(debug_assertions)]
    pub fn with_preview_scene(mut self, scene: &str) -> Self {
        match scene {
            "review" => self.review_drafts = true,
            "transfer" => {
                let progress = Arc::new(UploadProgress::default());
                progress.begin(0);
                progress.update(8 * 1024 * 1024, 51 * 1024 * 1024);
                self.upload_progress = Some(progress);
                self.busy = true;
            }
            _ => {}
        }
        self
    }
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
            upload_progress: None,
            review_drafts: false,
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
        self.upload_progress = None;
        self.review_drafts = false;
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
                this.upload_progress = None;
                this.cancelled = Arc::new(AtomicBool::new(false));
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
            ShareError::Remote(code)
                if code == "share_unavailable" || code == "share_session_rejected" =>
            {
                "share.error.network"
            }
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
        if self.busy
            || self.device.is_empty()
            || self.drafts.is_empty()
            || self.snapshot.as_ref().is_none_or(|s| !s.plan.storage_ready)
        {
            return;
        }
        let drafts = self.drafts.clone();
        let retention = self.retention;
        let progress = Arc::new(UploadProgress::default());
        self.upload_progress = Some(progress.clone());
        self.review_drafts = false;
        self.watch_upload(progress.clone(), cx);
        self.run(
            move |client| {
                let mut uploads = Vec::new();
                let mut warning = None;
                for (index, draft) in drafts.into_iter().enumerate() {
                    progress.begin(index);
                    match client.upload(&draft.upload, retention, |done, total| {
                        progress.update(done, total)
                    }) {
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
    /// Stage files handed over by the OS ("Share with Inklura", Send to,
    /// Finder Open With). They open in Review; nothing uploads until the
    /// user clicks Share.
    pub fn stage_external(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.busy {
            self.error = Some(t!("share.error.busy").to_string());
            cx.notify();
            return;
        }
        self.error = None;
        self.notice = None;
        self.add_paths_then(paths, true, cx);
    }
    fn add_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        self.add_paths_then(paths, false, cx);
    }
    fn add_paths_then(&mut self, paths: Vec<PathBuf>, review: bool, cx: &mut Context<Self>) {
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
                    Ok(drafts) => {
                        this.drafts.extend(drafts);
                        if review && !this.drafts.is_empty() {
                            this.review_drafts = true;
                        }
                    }
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
        self.capture_then(None, cx);
    }
    /// Tray/palette "Capture & share": select a region while the main window
    /// may still be hidden, then run `on_ready` (which shows the window) so
    /// the annotator, or the capture error, is what the user sees next.
    pub fn capture_external(&mut self, on_ready: CaptureReady, cx: &mut Context<Self>) {
        if self.busy {
            self.error = Some(t!("share.error.busy").to_string());
            cx.notify();
            on_ready(cx);
            return;
        }
        self.error = None;
        self.notice = None;
        self.capture_then(Some(on_ready), cx);
    }
    /// Tray/palette "Share files…": open the native picker straight away.
    pub fn choose_files_external(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            self.error = Some(t!("share.error.busy").to_string());
            cx.notify();
            return;
        }
        self.choose_files(cx);
    }
    fn capture_then(&mut self, on_ready: Option<CaptureReady>, cx: &mut Context<Self>) {
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
            if let Some(on_ready) = on_ready {
                let _ = cx.update(on_ready);
            }
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
}
impl Focusable for ShareView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Drop for ShareView {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}
mod progress;
mod rendering;
use progress::UploadProgress;

#[cfg(test)]
mod tests;
