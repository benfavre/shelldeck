//! Local image intake for hosted requests. Drafts stay in memory until the
//! request/comment is submitted; Manage-scoped tickets then upload them to
//! Inklura Share.

use crate::icons::lucide_icon;
use crate::scale::px;
use crate::theme::ShellDeckColors;
use gpui::prelude::*;
use gpui::{
    black, div, img, white, AnyElement, App, Context, ElementId, FocusHandle, Image, ImageFormat,
    KeyDownEvent, ObjectFit, Render, SharedString, Window,
};
use shelldeck_core::config::cloud_account;
use shelldeck_core::config::issues::{
    IssueAttachment, IssueAttachmentUpload, ISSUE_ATTACHMENT_MAX_BYTES,
};
use std::path::Path;
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;

use crate::t;

type AttachmentDeleteCallback = Rc<dyn Fn(usize, &mut App)>;

fn lightbox_icon_button(
    id: &'static str,
    icon: &'static str,
    size: f32,
    on_click: impl Fn(&mut App) + 'static,
) -> AnyElement {
    // IconButton currently derives its glyph color from the application theme;
    // that can turn black in a light theme and disappear on this intentionally
    // dark media surface, so the lightbox owns this tiny forced-white variant.
    div()
        .id(id)
        .flex_shrink_0()
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.0))
        .cursor_pointer()
        .hover(|style| style.bg(white().opacity(0.12)))
        .child(lucide_icon(icon, size * 0.48, white().opacity(0.88)))
        .on_click(move |_, _, cx| {
            cx.stop_propagation();
            on_click(cx);
        })
        .into_any_element()
}

#[derive(Clone, Debug)]
pub struct AttachmentDraft {
    pub filename: String,
    pub content_type: String,
    pub bytes: Arc<Vec<u8>>,
    pub image: Arc<Image>,
}

impl AttachmentDraft {
    pub fn from_bytes(filename: impl Into<String>, bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.is_empty() {
            return Err(t!("user.requests.attachments.error.empty").to_string());
        }
        if bytes.len() > ISSUE_ATTACHMENT_MAX_BYTES {
            return Err(t!("user.requests.attachments.error.too_large").to_string());
        }
        let (content_type, format, extension) = detect_image(&bytes)
            .ok_or_else(|| t!("user.requests.attachments.error.unsupported").to_string())?;
        let mut filename = filename.into();
        if filename.trim().is_empty() {
            filename = format!("capture.{extension}");
        }
        Ok(Self {
            filename,
            content_type: content_type.to_string(),
            image: Arc::new(Image::from_bytes(format, bytes.clone())),
            bytes: Arc::new(bytes),
        })
    }

    pub fn from_path(path: &Path) -> Result<Self, String> {
        let meta = std::fs::metadata(path).map_err(|e| {
            t!(
                "user.requests.attachments.error.read",
                error = e.to_string()
            )
            .to_string()
        })?;
        if meta.len() as usize > ISSUE_ATTACHMENT_MAX_BYTES {
            return Err(t!("user.requests.attachments.error.too_large").to_string());
        }
        let bytes = std::fs::read(path).map_err(|e| {
            t!(
                "user.requests.attachments.error.read",
                error = e.to_string()
            )
            .to_string()
        })?;
        let filename = path.file_name().and_then(|v| v.to_str()).unwrap_or("image");
        Self::from_bytes(filename, bytes)
    }

    pub fn upload(&self) -> IssueAttachmentUpload {
        IssueAttachmentUpload {
            filename: self.filename.clone(),
            content_type: self.content_type.clone(),
            bytes: self.bytes.as_ref().clone(),
        }
    }
}

fn display_bytes(bytes: usize) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} Mo", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} Ko", (bytes.saturating_add(1023)) / 1024)
    }
}

/// Shared visual draft gallery used by both request composers.
///
/// There is no adabraka media-card component; keeping the thumbnail, metadata
/// and remove affordance here prevents User and Support from drifting apart.
pub fn render_attachment_draft_gallery(
    drafts: &[AttachmentDraft],
    id_prefix: &'static str,
    on_remove: impl Fn(usize, &mut App) + Clone + 'static,
) -> AnyElement {
    let mut gallery = div().flex().flex_wrap().gap(px(8.0)).py(px(2.0));

    for (index, draft) in drafts.iter().enumerate() {
        let remove = on_remove.clone();
        gallery = gallery.child(
            div()
                .id(ElementId::from(SharedString::from(format!(
                    "{id_prefix}-{index}"
                ))))
                .relative()
                .w(px(112.0))
                .overflow_hidden()
                .rounded(px(8.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_primary())
                .hover(|style| style.border_color(ShellDeckColors::primary().opacity(0.5)))
                .child(
                    div()
                        .w_full()
                        .h(px(68.0))
                        .overflow_hidden()
                        .bg(ShellDeckColors::bg_surface())
                        .child(
                            img(draft.image.clone())
                                .size_full()
                                .object_fit(ObjectFit::Cover),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.0))
                        .px(px(7.0))
                        .py(px(5.0))
                        .child(
                            div()
                                .min_w(px(0.0))
                                .truncate()
                                .text_size(px(10.0))
                                .font_weight(gpui::FontWeight::MEDIUM)
                                .text_color(ShellDeckColors::text_primary())
                                .child(draft.filename.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(9.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(display_bytes(draft.bytes.len())),
                        ),
                )
                .child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "{id_prefix}-remove-{index}"
                        ))))
                        .absolute()
                        .top(px(5.0))
                        .right(px(5.0))
                        .size(px(20.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(black().opacity(0.72))
                        .cursor_pointer()
                        .hover(|style| style.bg(ShellDeckColors::error()))
                        .child(lucide_icon("x", 11.0, white()))
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            remove(index, cx);
                        }),
                ),
        );
    }

    gallery.into_any_element()
}

/// Shared persisted-image gallery for User requests and Support threads.
///
/// There is no adabraka component for a remote image attachment card, so this
/// one custom shape lives here and is reused by both surfaces.
pub fn render_stored_attachment_gallery(
    attachments: &[IssueAttachment],
    id_prefix: &'static str,
    on_open: impl Fn(usize, &mut App) + Clone + 'static,
    on_delete: Option<AttachmentDeleteCallback>,
) -> AnyElement {
    let mut gallery = div().flex().flex_wrap().gap(px(8.0)).pt(px(4.0));

    for (index, attachment) in attachments.iter().enumerate() {
        let image_url = attachment.url.clone();
        let filename = attachment.filename.clone();
        let open = on_open.clone();
        let delete = on_delete.clone();

        gallery = gallery.child(
            div()
                .id(ElementId::from(SharedString::from(format!(
                    "{id_prefix}-{}",
                    attachment.id
                ))))
                .relative()
                .w(px(148.0))
                .overflow_hidden()
                .rounded(px(8.0))
                .border_1()
                .border_color(ShellDeckColors::border())
                .bg(ShellDeckColors::bg_primary())
                .cursor_pointer()
                .hover(|style| {
                    style
                        .border_color(ShellDeckColors::primary().opacity(0.55))
                        .bg(ShellDeckColors::hover_bg())
                })
                .child(
                    div()
                        .w_full()
                        .h(px(92.0))
                        .overflow_hidden()
                        .bg(ShellDeckColors::bg_surface())
                        .child(
                            img(SharedString::from(image_url))
                                .size_full()
                                .object_fit(ObjectFit::Cover)
                                .with_fallback(|| {
                                    div()
                                        .size_full()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(lucide_icon(
                                            "eye",
                                            24.0,
                                            ShellDeckColors::text_muted(),
                                        ))
                                        .into_any_element()
                                }),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        .px(px(8.0))
                        .py(px(6.0))
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_primary())
                        .child(div().flex_1().min_w(px(0.0)).truncate().child(filename))
                        .child(lucide_icon(
                            "maximize-2",
                            12.0,
                            ShellDeckColors::text_muted(),
                        )),
                )
                .when_some(delete, |el, delete| {
                    el.child(
                        div()
                            .id(ElementId::from(SharedString::from(format!(
                                "{id_prefix}-delete-{}",
                                attachment.id
                            ))))
                            .absolute()
                            .top(px(6.0))
                            .right(px(6.0))
                            .size(px(26.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(7.0))
                            .bg(black().opacity(0.72))
                            .cursor_pointer()
                            .hover(|style| style.bg(ShellDeckColors::error()))
                            .child(lucide_icon("trash-2", 13.0, white()))
                            .on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                delete(index, cx);
                            }),
                    )
                })
                .on_click(move |_, _, cx| {
                    open(index, cx);
                }),
        );
    }

    gallery.into_any_element()
}

/// Native full-window image viewer for request threads.
///
/// adabraka-ui has dialogs and sheets but no media lightbox. This dedicated
/// view keeps the one custom full-screen shape shared by User and Support,
/// while preserving the surrounding thread and its scroll position.
pub struct AttachmentLightbox {
    attachments: Vec<IssueAttachment>,
    selected: usize,
    focus_handle: FocusHandle,
    focused: bool,
    on_close: Rc<dyn Fn(&mut App)>,
}

impl AttachmentLightbox {
    pub fn new(
        attachments: Vec<IssueAttachment>,
        selected: usize,
        on_close: impl Fn(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        let selected = selected.min(attachments.len().saturating_sub(1));
        Self {
            attachments,
            selected,
            focus_handle: cx.focus_handle(),
            focused: false,
            on_close: Rc::new(on_close),
        }
    }

    fn close(&self, cx: &mut App) {
        (self.on_close)(cx);
    }

    fn select_previous(&mut self, cx: &mut Context<Self>) {
        if self.attachments.len() > 1 {
            self.selected = if self.selected == 0 {
                self.attachments.len() - 1
            } else {
                self.selected - 1
            };
            cx.notify();
        }
    }

    fn select_next(&mut self, cx: &mut Context<Self>) {
        if self.attachments.len() > 1 {
            self.selected = (self.selected + 1) % self.attachments.len();
            cx.notify();
        }
    }

    fn handle_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event.keystroke.key.as_str() {
            "escape" => self.close(cx),
            "left" => self.select_previous(cx),
            "right" => self.select_next(cx),
            _ => {}
        }
    }
}

impl Render for AttachmentLightbox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.focused {
            window.focus(&self.focus_handle);
            self.focused = true;
        }

        let Some(current) = self.attachments.get(self.selected).cloned() else {
            return div().into_any_element();
        };
        let has_multiple = self.attachments.len() > 1;
        let counter = format!("{} / {}", self.selected + 1, self.attachments.len());
        let original_url = if current.viewer_url.is_empty() {
            current.url.clone()
        } else {
            current.viewer_url.clone()
        };

        let close_entity = cx.entity();
        let previous_entity = close_entity.clone();
        let next_entity = close_entity.clone();

        let mut thumbnails = div()
            .id("attachment-lightbox-thumbnails")
            .flex()
            .items_center()
            .justify_center()
            .gap(px(7.0))
            .overflow_x_scroll()
            .px(px(16.0))
            .py(px(10.0));
        for (index, attachment) in self.attachments.iter().enumerate() {
            let entity = close_entity.clone();
            let selected = index == self.selected;
            thumbnails = thumbnails.child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "attachment-lightbox-thumb-{}",
                        attachment.id
                    ))))
                    .flex_shrink_0()
                    .w(px(58.0))
                    .h(px(42.0))
                    .p(px(2.0))
                    .rounded(px(7.0))
                    .border_1()
                    .border_color(if selected {
                        white()
                    } else {
                        white().opacity(0.18)
                    })
                    .bg(if selected {
                        white().opacity(0.12)
                    } else {
                        black().opacity(0.2)
                    })
                    .cursor_pointer()
                    .hover(|style| style.border_color(white().opacity(0.7)))
                    .child(
                        div().size_full().overflow_hidden().rounded(px(4.0)).child(
                            img(SharedString::from(attachment.url.clone()))
                                .size_full()
                                .object_fit(ObjectFit::Cover),
                        ),
                    )
                    .on_click(move |_, _, cx| {
                        entity.update(cx, |this, cx| {
                            this.selected = index;
                            cx.notify();
                        });
                    }),
            );
        }

        div()
            .id("attachment-lightbox")
            .absolute()
            .inset_0()
            .occlude()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .flex()
            .flex_col()
            .bg(black().opacity(0.94))
            .text_color(white())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(18.0))
                    .py(px(12.0))
                    .border_b_1()
                    .border_color(white().opacity(0.1))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(2.0))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.0))
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .child(current.filename),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(white().opacity(0.62))
                                    .child(counter),
                            ),
                    )
                    .child(lightbox_icon_button(
                        "attachment-lightbox-original",
                        "external-link",
                        32.0,
                        move |_| {
                            let _ = cloud_account::open_in_browser(&original_url);
                        },
                    ))
                    .child(lightbox_icon_button(
                        "attachment-lightbox-close",
                        "x",
                        32.0,
                        move |cx| {
                            close_entity.update(cx, |this, cx| this.close(cx));
                        },
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .items_center()
                    .gap(px(12.0))
                    .px(px(18.0))
                    .py(px(12.0))
                    .when(has_multiple, |el| {
                        el.child(lightbox_icon_button(
                            "attachment-lightbox-previous",
                            "chevron-left",
                            38.0,
                            move |cx| {
                                previous_entity.update(cx, |this, cx| this.select_previous(cx));
                            },
                        ))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                img(SharedString::from(current.url))
                                    .size_full()
                                    .object_fit(ObjectFit::Contain)
                                    .with_fallback(|| {
                                        div()
                                            .flex()
                                            .flex_col()
                                            .items_center()
                                            .gap(px(8.0))
                                            .text_color(white().opacity(0.6))
                                            .child(lucide_icon("eye-off", 28.0, white()))
                                            .child("Impossible de charger l’image")
                                            .into_any_element()
                                    }),
                            ),
                    )
                    .when(has_multiple, |el| {
                        el.child(lightbox_icon_button(
                            "attachment-lightbox-next",
                            "chevron-right",
                            38.0,
                            move |cx| {
                                next_entity.update(cx, |this, cx| this.select_next(cx));
                            },
                        ))
                    }),
            )
            .when(has_multiple, |el| el.child(thumbnails))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .pb(px(10.0))
                    .text_size(px(10.0))
                    .text_color(white().opacity(0.5))
                    .child("← → pour naviguer · Échap pour fermer"),
            )
            .into_any_element()
    }
}

pub fn draft_from_clipboard_image(image: &Image) -> Result<AttachmentDraft, String> {
    let extension = match image.format {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpg",
        ImageFormat::Webp => "webp",
        _ => return Err(t!("user.requests.attachments.error.clipboard_format").to_string()),
    };
    AttachmentDraft::from_bytes(
        format!("capture-presse-papiers.{extension}"),
        image.bytes.clone(),
    )
}

fn detect_image(bytes: &[u8]) -> Option<(&'static str, ImageFormat, &'static str)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some(("image/png", ImageFormat::Png, "png"))
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(("image/jpeg", ImageFormat::Jpeg, "jpg"))
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(("image/webp", ImageFormat::Webp, "webp"))
    } else {
        None
    }
}

/// Opens the platform's interactive area selector and returns a PNG draft.
/// Commands are invoked directly (no shell interpolation).
pub fn capture_region() -> Result<AttachmentDraft, String> {
    let file = tempfile::Builder::new()
        .prefix("shelldeck-capture-")
        .suffix(".png")
        .tempfile()
        .map_err(|e| {
            t!(
                "user.requests.attachments.error.capture",
                error = e.to_string()
            )
            .to_string()
        })?;
    let path = file.path().to_path_buf();
    file.close().map_err(|e| {
        t!(
            "user.requests.attachments.error.capture",
            error = e.to_string()
        )
        .to_string()
    })?;

    #[cfg(target_os = "macos")]
    let captured = Command::new("/usr/sbin/screencapture")
        .args(["-i", "-x"])
        .arg(&path)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);

    #[cfg(target_os = "windows")]
    let captured = {
        let escaped = path.to_string_lossy().replace('\'', "''");
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; \
             Add-Type -TypeDefinition 'using System.Runtime.InteropServices; public static class ShellDeckClipboard {{ [DllImport(\"user32.dll\")] public static extern uint GetClipboardSequenceNumber(); }}'; \
             $before=[ShellDeckClipboard]::GetClipboardSequenceNumber(); Start-Process 'ms-screenclip:'; \
             $end=(Get-Date).AddSeconds(90); do {{ Start-Sleep -Milliseconds 250; \
             $current=[ShellDeckClipboard]::GetClipboardSequenceNumber(); \
             if ($current -ne $before -and [Windows.Forms.Clipboard]::ContainsImage()) {{ \
             $i=[Windows.Forms.Clipboard]::GetImage(); $i.Save('{escaped}',[Drawing.Imaging.ImageFormat]::Png); exit 0 }} \
             }} while ((Get-Date) -lt $end); exit 1"
        );
        Command::new("powershell.exe")
            .args(["-NoProfile", "-STA", "-Command", &script])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };

    #[cfg(target_os = "linux")]
    let captured = {
        let attempts: &[(&str, &[&str])] = &[
            ("gnome-screenshot", &["-a", "-f"]),
            ("spectacle", &["-r", "-b", "-n", "-o"]),
            ("import", &[]),
        ];
        // Only try tools that are actually installed: when none is, the
        // failure is a missing dependency, not a user cancellation, and it
        // deserves its own message instead of "capture cancelled".
        let installed: Vec<(&str, &[&str])> = attempts
            .iter()
            .copied()
            .filter(|(program, _)| shelldeck_core::util::executable_on_path(program))
            .collect();
        if installed.is_empty() {
            return Err(t!("attachments.capture.tool_missing").to_string());
        }
        installed.iter().any(|(program, args)| {
            Command::new(program)
                .args(*args)
                .arg(&path)
                .status()
                .map(|s| s.success() && path.metadata().map(|m| m.len() > 0).unwrap_or(false))
                .unwrap_or(false)
        })
    };

    if !captured {
        return Err(t!("user.requests.attachments.error.capture_cancelled").to_string());
    }
    AttachmentDraft::from_path(&path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_extension_spoofing() {
        assert!(AttachmentDraft::from_bytes("fake.png", b"not a png".to_vec()).is_err());
    }

    #[test]
    fn recognizes_png_magic() {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.extend_from_slice(&[0; 16]);
        let draft = AttachmentDraft::from_bytes("shot.png", bytes).unwrap();
        assert_eq!(draft.content_type, "image/png");
    }
}
