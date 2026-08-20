//! The composer's two footer affordances: `+` (attachments) and `@` (mentions).
//!
//! Both shipped as drawn-but-inert placeholders. This module makes them real
//! without changing how a turn is sent: the assistant still emits one
//! `AiAssistantEvent::Submit` carrying one `AiContext`, now with `mentions` and
//! `attachments` populated.
//!
//! The design decisions and the scoping rules live in `docs/ai-mentions.md`.
//! Two of them shape everything here:
//!
//! * **The draft is the source of truth for mentions.** The chip row is a view
//!   of the text, not a second list. Deleting `@prod-web-01` from the sentence
//!   removes the mention, which is the only rule a user can predict without
//!   being taught one.
//! * **Attachments are refused, never silently dropped.** A CLI backend cannot
//!   receive an image; the menu says so before the file dialog opens, and the
//!   core layer rejects the turn if a backend switch made a staged image
//!   undeliverable.

use adabraka_ui::components::icon_source::IconSource;
use adabraka_ui::components::input_state::TextHighlight;
use adabraka_ui::prelude::{Button, ButtonSize, ButtonVariant};
use gpui::prelude::*;
use gpui::{
    div, hsla, point, px as gpui_px, AnyElement, AsyncApp, BoxShadow, ClipboardEntry, Context,
    Image, MouseButton, SharedString, Window,
};
use shelldeck_core::ai::{
    filter_mention_candidates, insert_mention, mention_query_at_caret, mention_spans,
    reconcile_mentions, remove_mention_token, validate_attachments, AiAttachment, AiAttachmentKind,
    AiMention, AttachmentError, MentionCandidate, MentionKind, MentionRef, AI_ATTACHMENT_MAX_COUNT,
    MENTION_PICKER_LIMIT,
};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use super::{AiAssistantEvent, AiAssistantView};
use crate::attachment_annotator::AttachmentAnnotator;
use crate::icons::{lucide_icon, lucide_path};
use crate::issue_attachments::{
    capture_region, draft_from_clipboard_image, AttachmentDraft, AttachmentLightbox, LightboxItem,
};
use crate::scale::px;
use crate::t;
use crate::theme::ShellDeckColors;

/// A staged attachment plus, for images, the decoded bitmap used to preview it.
///
/// The preview is kept beside the wire value rather than derived from it: the
/// wire value carries base64, and re-decoding it on every frame to paint a chip
/// would be absurd. Every intake path already holds the decoded image (the
/// shared `AttachmentDraft` carries one), so this costs nothing to fill.
pub(super) struct StagedAttachment {
    pub(super) attachment: AiAttachment,
    pub(super) preview: Option<Arc<Image>>,
}

/// An open `@` picker, anchored to the partial token being typed.
///
/// There is no separate search field: clicking `@` inserts an `@` at the caret
/// and the picker reads the query straight out of the draft. One filtering path
/// for both entry points, and no focus hand-off between two inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MentionPicker {
    /// Byte offset of the `@` in the draft.
    pub(super) at: usize,
    /// What has been typed after it.
    pub(super) query: String,
}

impl AiAssistantView {
    // ---------------------------------------------------------------- state

    /// Push the directory the Workspace built for the current account, mode and
    /// active site. Both hosts get it; the Dock running before the Workspace
    /// exists simply never receives one and says so.
    pub fn set_mention_directory(
        &mut self,
        directory: Rc<Vec<MentionCandidate>>,
        cx: &mut Context<Self>,
    ) {
        self.mention_directory = directory;
        self.mention_directory_ready = true;
        cx.notify();
    }

    // ------------------------------------------------------------- mentions

    /// The `@` button: insert the trigger character at the caret and let the
    /// draft-change path open the picker, exactly as typing `@` would.
    pub(super) fn begin_mention(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loading || !self.available {
            return;
        }
        cx.emit(AiAssistantEvent::RefreshMentions);
        if !self.mention_directory_ready {
            self.set_notice(t!("ai.mention.unavailable").to_string(), cx);
            return;
        }
        if self.mention_directory.is_empty() {
            self.set_notice(t!("ai.mention.empty").to_string(), cx);
            return;
        }
        let state = self.prompt_state.clone();
        let (draft, caret) = {
            let read = state.read(cx);
            (read.content().to_string(), read.caret_offset())
        };
        let caret = clamp_boundary(&draft, caret);
        let needs_space = caret > 0
            && !draft[..caret]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace);
        let insertion = if needs_space { " @" } else { "@" };
        let mut next = String::with_capacity(draft.len() + insertion.len());
        next.push_str(&draft[..caret]);
        next.push_str(insertion);
        let new_caret = next.len();
        next.push_str(&draft[caret..]);
        state.update(cx, |state, cx| {
            state.replace_content_with_caret(next, new_caret, cx);
        });
        state.read(cx).focus_handle(cx).focus(window);
        self.sync_mention_picker(cx);
    }

    /// Re-derive the picker from the draft. Called on every composer change.
    ///
    /// The picker follows the text: it opens when the caret sits inside an
    /// `@query`, closes when that query disappears (the `@` was deleted, a
    /// newline was typed, the caret moved away) and closes when nothing matches
    /// any more — so a sentence that merely contains an `@` does not leave a
    /// panel hanging over the conversation.
    pub(super) fn sync_mention_picker(&mut self, cx: &mut Context<Self>) {
        let (draft, caret) = {
            let state = self.prompt_state.read(cx);
            (state.content().to_string(), state.caret_offset())
        };
        // Colouring is independent of the picker: a draft keeps its coloured
        // mentions long after the picker closed, and must lose them the moment
        // the text stops carrying them.
        self.sync_mention_highlights(&draft, cx);
        if !self.mention_directory_ready || self.mention_directory.is_empty() {
            if self.mention_picker.take().is_some() {
                cx.notify();
            }
            return;
        }
        // A token that was just completed still *looks* like a query — the
        // draft ends in `@prod-web-01 `. Re-opening the picker on it would put
        // a panel back over the conversation the instant the user accepted a
        // row. Editing that text clears the suppression and offers completion
        // again.
        if self.mention_completed_draft.as_deref() == Some(draft.as_str()) {
            if self.mention_picker.take().is_some() {
                cx.notify();
            }
            return;
        }
        self.mention_completed_draft = None;
        let next = mention_query_at_caret(&draft, caret).and_then(|(at, query)| {
            let has_match =
                !filter_mention_candidates(&self.mention_directory, &query, 1).is_empty();
            has_match.then_some(MentionPicker { at, query })
        });
        if next != self.mention_picker {
            // Opening is the moment the rows must be current; a keystroke that
            // only narrows an already-open query is not.
            if self.mention_picker.is_none() && next.is_some() {
                cx.emit(AiAssistantEvent::RefreshMentions);
            }
            self.mention_picker = next;
            cx.notify();
        }
    }

    /// Repaint the `@Label` tokens the draft still carries.
    ///
    /// Driven by the same change pass as the picker, so the colour appears on
    /// the keystroke that completes a mention and disappears on the one that
    /// breaks it. Only *live* references are painted: text that merely looks
    /// like a mention gets no colour, which is precisely the signal — the
    /// colour means "this one resolved", not "this one has an @".
    fn sync_mention_highlights(&mut self, draft: &str, cx: &mut Context<Self>) {
        let labels: Vec<String> = reconcile_mentions(draft, &self.mentions)
            .into_iter()
            .map(|reference| reference.label)
            .collect();
        // Accent colour on the text, same hue at low opacity behind it — the
        // convention every chat application uses, and the reason a mention is
        // recognisable at a glance rather than read word by word.
        let color = ShellDeckColors::primary();
        let background = ShellDeckColors::primary().opacity(0.14);
        let spans: Vec<TextHighlight> = mention_spans(draft, &labels)
            .into_iter()
            .map(|span| TextHighlight::new(span, color).background(background))
            .collect();
        self.prompt_state
            .update(cx, |state, cx| state.set_highlights(spans, cx));
    }

    /// Rows currently offered, already ranked.
    pub(super) fn mention_matches(&self) -> Vec<&MentionCandidate> {
        let Some(picker) = self.mention_picker.as_ref() else {
            return Vec::new();
        };
        filter_mention_candidates(&self.mention_directory, &picker.query, MENTION_PICKER_LIMIT)
    }

    /// Accept a candidate: replace the partial `@query` with its token and
    /// record the reference.
    pub(super) fn accept_mention(&mut self, reference: MentionRef, cx: &mut Context<Self>) {
        let state = self.prompt_state.clone();
        let (draft, caret) = {
            let read = state.read(cx);
            (read.content().to_string(), read.caret_offset())
        };
        let (next, new_caret) = insert_mention(&draft, caret, &reference.label);
        self.mention_completed_draft = Some(next.clone());
        state.update(cx, |state, cx| {
            state.replace_content_with_caret(next, new_caret, cx);
        });
        self.mentions.push(reference);
        self.mention_picker = None;
        cx.notify();
    }

    /// True when Enter must complete a mention instead of sending the turn.
    pub(super) fn mention_picker_intercepts_commit(&self) -> bool {
        self.mention_picker.is_some()
    }

    /// Enter with the picker open takes the top-ranked row. The list is ranked
    /// and stable, so the first row is the one the user is looking at.
    pub(super) fn accept_top_mention(&mut self, cx: &mut Context<Self>) {
        let Some(reference) = self.mention_matches().first().map(|row| row.as_ref()) else {
            self.mention_picker = None;
            cx.notify();
            return;
        };
        self.accept_mention(reference, cx);
    }

    /// Chip `×`: drop the reference and one occurrence of its token.
    pub(super) fn remove_mention(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.mentions.len() {
            return;
        }
        let removed = self.mentions.remove(index);
        let state = self.prompt_state.clone();
        let draft = state.read(cx).content().to_string();
        let next = remove_mention_token(&draft, &removed.label);
        if next != draft {
            let caret = next.len();
            state.update(cx, |state, cx| {
                state.replace_content_with_caret(next, caret, cx);
            });
        }
        cx.notify();
    }

    /// Resolve what will actually be sent.
    ///
    /// Two filters, both deliberate: the draft decides which references
    /// survive, and the live directory decides which of those still resolve —
    /// a draft can outlive a site switch, and a mention that left the caller's
    /// scope must not travel with the turn.
    pub(super) fn resolved_mentions(&self, draft: &str) -> Vec<AiMention> {
        reconcile_mentions(draft, &self.mentions)
            .into_iter()
            .filter_map(|reference| {
                self.mention_directory
                    .iter()
                    .find(|candidate| {
                        candidate.kind == reference.kind && candidate.id == reference.id
                    })
                    .map(MentionCandidate::to_mention)
            })
            .collect()
    }

    /// Drop the composer's mention/attachment state once a turn has left.
    pub(super) fn clear_composer_payload(&mut self) {
        self.mentions.clear();
        self.attachments.clear();
        self.mention_picker = None;
        self.attach_menu_open = false;
        self.attachment_error = None;
    }

    // ---------------------------------------------------------- attachments

    pub(super) fn toggle_attach_menu(&mut self, cx: &mut Context<Self>) {
        if self.loading || !self.available {
            return;
        }
        self.attach_menu_open = !self.attach_menu_open;
        self.attachment_error = None;
        cx.notify();
    }

    /// Whether the active backend can carry image bytes at all.
    pub(super) fn backend_takes_images(&self) -> bool {
        self.backend.supports_image_attachments()
    }

    fn push_attachment(
        &mut self,
        attachment: AiAttachment,
        preview: Option<Arc<Image>>,
        cx: &mut Context<Self>,
    ) {
        if self.attachments.len() >= AI_ATTACHMENT_MAX_COUNT {
            self.report_attachment_error(AttachmentError::TooMany, cx);
            return;
        }
        if let Err(error) = validate_attachments(self.backend, std::slice::from_ref(&attachment)) {
            self.report_attachment_error(error, cx);
            return;
        }
        self.attachments.push(StagedAttachment {
            attachment,
            preview,
        });
        self.attachment_error = None;
        self.attach_menu_open = false;
        cx.notify();
    }

    /// What actually travels with the turn.
    pub(super) fn staged_attachments(&self) -> Vec<AiAttachment> {
        self.attachments
            .iter()
            .map(|staged| staged.attachment.clone())
            .collect()
    }

    /// Open the shared image viewer on a staged attachment.
    ///
    /// The same component the request threads use, so checking what you are
    /// about to send looks exactly like re-reading what you already sent.
    pub(super) fn open_attachment_preview(&mut self, index: usize, cx: &mut Context<Self>) {
        let items: Vec<(usize, LightboxItem)> = self
            .attachments
            .iter()
            .enumerate()
            .filter_map(|(position, staged)| {
                let preview = staged.preview.clone()?;
                Some((
                    position,
                    LightboxItem::from_image(
                        format!("ai-attachment-{position}"),
                        staged.attachment.name.clone(),
                        preview,
                    ),
                ))
            })
            .collect();
        if items.is_empty() {
            return;
        }
        // The clicked chip may not be the first previewable one, so the
        // selection is the position *within the previewable subset*.
        let selected = items
            .iter()
            .position(|(position, _)| *position == index)
            .unwrap_or(0);
        let parent = cx.entity().downgrade();
        let lightbox = cx.new(|cx| {
            AttachmentLightbox::new(
                items.into_iter().map(|(_, item)| item).collect(),
                selected,
                move |cx| {
                    if let Some(parent) = parent.upgrade() {
                        parent.update(cx, |this, cx| {
                            this.attachment_lightbox = None;
                            cx.notify();
                        });
                    }
                },
                cx,
            )
        });
        self.attachment_lightbox = Some(lightbox);
        cx.notify();
    }

    pub(super) fn render_attachment_lightbox(&self) -> Option<AnyElement> {
        self.attachment_lightbox
            .as_ref()
            .map(|lightbox| lightbox.clone().into_any_element())
    }

    fn report_attachment_error(&mut self, error: AttachmentError, cx: &mut Context<Self>) {
        self.attachment_error = Some(match error {
            AttachmentError::TooLarge { limit } => t!(
                "ai.attachment.error.too_large",
                limit = format!("{} Mo", limit / (1024 * 1024))
            )
            .to_string(),
            AttachmentError::TooMany => t!(
                "ai.attachment.error.too_many",
                max = AI_ATTACHMENT_MAX_COUNT
            )
            .to_string(),
            AttachmentError::UnsupportedByBackend => t!(
                "ai.attachment.error.backend",
                backend = self.backend.display_name()
            )
            .to_string(),
            other => t!(other.message_key()).to_string(),
        });
        self.attach_menu_open = false;
        cx.notify();
    }

    pub(super) fn remove_attachment(&mut self, index: usize, cx: &mut Context<Self>) {
        if index < self.attachments.len() {
            self.attachments.remove(index);
            self.attachment_error = None;
            cx.notify();
        }
    }

    /// File picker. Accepts anything; the kind is decided from the bytes, so a
    /// `.log` that is really a screenshot is handled as one.
    pub(super) fn pick_attachment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.attach_menu_open = false;
        let receiver = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(t!("ai.attachment.choose").to_string().into()),
            starting_directory: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                return;
            };
            let _ = this.update(cx, |view, cx| view.import_attachment_paths(paths, cx));
        })
        .detach();
    }

    fn import_attachment_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        for path in paths {
            // The shared draft loader decodes images and rejects anything that
            // is not one, which is exactly the discrimination needed here: an
            // image gets a preview, everything else goes down the text path.
            if let Ok(draft) = AttachmentDraft::from_path(&path) {
                match AiAttachment::image(
                    draft.filename.clone(),
                    draft.content_type.clone(),
                    &draft.bytes,
                ) {
                    Ok(attachment) => self.push_attachment(attachment, Some(draft.image), cx),
                    Err(error) => {
                        self.report_attachment_error(error, cx);
                        return;
                    }
                }
                continue;
            }
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| path.to_string_lossy().to_string());
            match std::fs::read(&path) {
                Ok(bytes) => match AiAttachment::from_bytes(name, bytes) {
                    Ok(attachment) => self.push_attachment(attachment, None, cx),
                    Err(error) => {
                        self.report_attachment_error(error, cx);
                        return;
                    }
                },
                Err(error) => {
                    self.attachment_error =
                        Some(t!("ai.attachment.error.read", error = error.to_string()).to_string());
                    cx.notify();
                    return;
                }
            }
        }
    }

    /// Clipboard intake. An image when the clipboard holds one, otherwise the
    /// text — the same `+` entry covers both because the user's intent ("send
    /// what I just copied") is the same.
    pub(super) fn paste_attachment(&mut self, cx: &mut Context<Self>) {
        self.attach_menu_open = false;
        let Some(item) = cx.read_from_clipboard() else {
            self.attachment_error = Some(t!("ai.attachment.error.clipboard").to_string());
            cx.notify();
            return;
        };
        let image = item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image.clone()),
            _ => None,
        });
        if let Some(image) = image {
            match draft_from_clipboard_image(&image) {
                Ok(draft) => {
                    match AiAttachment::image(
                        draft.filename.clone(),
                        draft.content_type.clone(),
                        &draft.bytes,
                    ) {
                        Ok(attachment) => self.push_attachment(attachment, Some(draft.image), cx),
                        Err(error) => self.report_attachment_error(error, cx),
                    }
                }
                Err(error) => {
                    self.attachment_error = Some(error);
                    cx.notify();
                }
            }
            return;
        }
        match item.text().filter(|text| !text.trim().is_empty()) {
            Some(text) => match AiAttachment::from_bytes(
                t!("ai.attachment.clipboard_name").to_string(),
                text.into_bytes(),
            ) {
                Ok(attachment) => self.push_attachment(attachment, None, cx),
                Err(error) => self.report_attachment_error(error, cx),
            },
            None => {
                self.attachment_error = Some(t!("ai.attachment.error.clipboard").to_string());
                cx.notify();
            }
        }
    }

    /// Interactive region capture, reusing the *whole* helper chain the
    /// request and ticket composers use: `capture_region` to select the area,
    /// then the annotation editor before the image is staged.
    ///
    /// The annotator is not a flourish. A screenshot sent to an assistant
    /// almost always needs "this bit here" pointed at — the arrow is the
    /// question. Skipping it, as the first cut of this method did, made the
    /// assistant the only surface in the application where a capture could not
    /// be annotated, for no reason other than that it was wired later.
    pub(super) fn capture_attachment(&mut self, cx: &mut Context<Self>) {
        self.attach_menu_open = false;
        if self.attachment_busy {
            return;
        }
        self.attachment_busy = true;
        cx.notify();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let result = cx
                .background_executor()
                .spawn(async { capture_region() })
                .await;
            let _ = this.update(cx, |view, cx| {
                view.attachment_busy = false;
                match result {
                    Ok(draft) => view.open_capture_annotator(draft, cx),
                    Err(error) => {
                        view.attachment_error = Some(error);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

    /// Present the shared annotation editor over the assistant surface.
    ///
    /// Hosted by the view rather than by the Workspace so it works in both
    /// hosts — the Dock is its own window and has no Workspace overlay to
    /// borrow.
    fn open_capture_annotator(&mut self, draft: AttachmentDraft, cx: &mut Context<Self>) {
        let cancel_parent = cx.entity().downgrade();
        let apply_parent = cancel_parent.clone();
        let annotator = cx.new(|cx| {
            AttachmentAnnotator::new(
                draft,
                move |cx| {
                    if let Some(parent) = cancel_parent.upgrade() {
                        parent.update(cx, |this, cx| {
                            this.capture_annotator = None;
                            cx.notify();
                        });
                    }
                },
                move |draft, cx| {
                    if let Some(parent) = apply_parent.upgrade() {
                        parent.update(cx, |this, cx| {
                            this.capture_annotator = None;
                            match AiAttachment::image(
                                draft.filename.clone(),
                                draft.content_type.clone(),
                                &draft.bytes,
                            ) {
                                Ok(attachment) => {
                                    this.push_attachment(attachment, Some(draft.image), cx)
                                }
                                Err(error) => this.report_attachment_error(error, cx),
                            }
                        });
                    }
                },
                cx,
            )
        });
        self.capture_annotator = Some(annotator);
        cx.notify();
    }

    /// The annotation editor, when one is open. Painted last so it covers the
    /// conversation and its own picker/menu overlays.
    pub(super) fn render_capture_annotator(&self) -> Option<AnyElement> {
        self.capture_annotator
            .as_ref()
            .map(|annotator| annotator.clone().into_any_element())
    }

    // ------------------------------------------------------------ rendering

    /// Chips shown above the field, beside the context chip: one per staged
    /// attachment, one per live mention.
    pub(super) fn render_composer_chips(
        &self,
        draft: &str,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut chips: Vec<AnyElement> = Vec::new();
        for (index, staged) in self.attachments.iter().enumerate() {
            let attachment = &staged.attachment;
            let undeliverable = attachment.is_image() && !self.backend_takes_images();
            let tint = if undeliverable {
                ShellDeckColors::error()
            } else {
                ShellDeckColors::text_muted()
            };
            let previewable = staged.preview.is_some();
            let mut chip = chip_shell(("ai-attachment-chip", index))
                .child(lucide_icon(attachment.kind.icon(), 11.0, tint))
                .child(
                    div()
                        .truncate()
                        .max_w(px(160.0))
                        .text_color(tint)
                        .child(attachment.name.clone()),
                );
            // An image chip is a door to the image. Without this the only way
            // to check what is about to be sent is to send it.
            if previewable {
                chip = chip
                    .cursor_pointer()
                    .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.open_attachment_preview(index, cx)),
                    );
            }
            chips.push(
                chip.child(chip_remove(
                    ("ai-attachment-drop", index),
                    cx.listener(move |this, _, _, cx| this.remove_attachment(index, cx)),
                ))
                .into_any_element(),
            );
        }
        // Only mentions the draft still carries get a chip: the row must show
        // what will be sent, not what was once picked.
        for (index, reference) in reconcile_mentions(draft, &self.mentions)
            .into_iter()
            .enumerate()
        {
            let position = self
                .mentions
                .iter()
                .position(|existing| existing == &reference)
                .unwrap_or(index);
            chips.push(
                chip_shell(("ai-mention-chip", index))
                    .child(lucide_icon(
                        reference.kind.icon(),
                        11.0,
                        ShellDeckColors::primary(),
                    ))
                    .child(
                        div()
                            .truncate()
                            .max_w(px(160.0))
                            .child(reference.label.clone()),
                    )
                    .child(chip_remove(
                        ("ai-mention-drop", index),
                        cx.listener(move |this, _, _, cx| this.remove_mention(position, cx)),
                    ))
                    .into_any_element(),
            );
        }
        chips
    }

    /// The `+` control and its menu trigger.
    pub(super) fn render_attach_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("ai-composer-attach", "")
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("plus"))
            .tooltip(t!("ai.attachment.tooltip").to_string())
            .on_click(cx.listener(|this, _, _, cx| this.toggle_attach_menu(cx)))
            .into_any_element()
    }

    /// The `@` control.
    pub(super) fn render_mention_button(&self, cx: &mut Context<Self>) -> AnyElement {
        Button::new("ai-composer-target", "")
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::Sm)
            .icon(IconSource::from("at-sign"))
            .tooltip(t!("ai.mention.tooltip").to_string())
            .on_click(cx.listener(|this, _, window, cx| this.begin_mention(window, cx)))
            .into_any_element()
    }

    /// The `+` dropdown. Image entries are disabled — with the reason on the
    /// row — when the backend cannot carry an image.
    pub(super) fn render_attach_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.attach_menu_open {
            return None;
        }
        let images_ok = self.backend_takes_images();
        let (left, _right) = composer_panel_bounds(self.host, self.history_open);
        let mut menu = floating_panel("ai-attach-menu")
            .bottom(px(78.0))
            .left(px(left))
            .w(px(268.0));

        menu = menu.child(menu_row(
            "ai-attach-file",
            "paperclip",
            t!("ai.attachment.menu.file").to_string(),
            Some(t!("ai.attachment.menu.file_hint").to_string()),
            true,
            cx.listener(|this, _, window, cx| this.pick_attachment(window, cx)),
        ));
        menu = menu.child(menu_row(
            "ai-attach-clipboard",
            "clipboard-paste",
            t!("ai.attachment.menu.clipboard").to_string(),
            None,
            true,
            cx.listener(|this, _, _, cx| this.paste_attachment(cx)),
        ));
        menu = menu.child(menu_row(
            "ai-attach-capture",
            "scan",
            t!("ai.attachment.menu.capture").to_string(),
            (!images_ok).then(|| {
                t!(
                    "ai.attachment.menu.no_images",
                    backend = self.backend.display_name()
                )
                .to_string()
            }),
            images_ok,
            cx.listener(|this, _, _, cx| this.capture_attachment(cx)),
        ));

        if !images_ok {
            menu = menu.child(
                div()
                    .px(px(9.0))
                    .pt(px(6.0))
                    .text_size(px(10.5))
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        t!(
                            "ai.attachment.menu.text_only",
                            backend = self.backend.display_name()
                        )
                        .to_string(),
                    ),
            );
        }

        Some(
            dismiss_layer(
                "ai-attach-backdrop",
                cx.listener(|this, _e, _window, cx| {
                    this.attach_menu_open = false;
                    cx.notify();
                }),
            )
            .child(menu)
            .into_any_element(),
        )
    }

    /// The `@` picker, grouped by kind.
    pub(super) fn render_mention_picker(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.mention_picker.as_ref()?;
        let matches = self.mention_matches();
        if matches.is_empty() {
            return None;
        }
        let (left, right) = composer_panel_bounds(self.host, self.history_open);
        let mut panel = floating_panel("ai-mention-picker")
            .bottom(px(78.0))
            .left(px(left))
            .right(px(right))
            .max_h(px(320.0))
            .overflow_y_scroll();

        let mut current_kind: Option<MentionKind> = None;
        for (index, candidate) in matches.iter().enumerate() {
            if current_kind != Some(candidate.kind) {
                current_kind = Some(candidate.kind);
                panel = panel.child(
                    div()
                        .px(px(9.0))
                        .pt(px(if index == 0 { 2.0 } else { 8.0 }))
                        .pb(px(3.0))
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!(candidate.kind.label_key()).to_string()),
                );
            }
            let reference = candidate.as_ref();
            let selected = index == 0;
            let mut row = div()
                .id(("ai-mention-row", index))
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(9.0))
                .py(px(6.0))
                .rounded(px(7.0))
                .cursor_pointer()
                .hover(|style| style.bg(ShellDeckColors::hover_bg()))
                .child(lucide_icon(
                    candidate.kind.icon(),
                    13.0,
                    ShellDeckColors::text_muted(),
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w(px(0.0))
                        .child(
                            div()
                                .truncate()
                                .text_size(px(12.0))
                                .text_color(ShellDeckColors::text_primary())
                                .child(candidate.label.clone()),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(px(10.5))
                                .text_color(ShellDeckColors::text_muted())
                                .child(candidate.sublabel.clone()),
                        ),
                );
            // Staff see rows from several sites at once; without the badge the
            // scope of what they just referenced would be invisible. A Site row
            // is its own site, so repeating it there says nothing.
            if candidate.kind != MentionKind::Site {
                if let Some(site) = candidate.site_label.clone() {
                    row = row.child(
                        // Capped and truncated: site labels are
                        // "Tenant — Site" and run long enough to paint over the
                        // row they annotate (`.agents/overflow.md`).
                        div()
                            .flex_shrink_0()
                            .max_w(px(120.0))
                            .truncate()
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(4.0))
                            .bg(ShellDeckColors::primary().opacity(0.12))
                            .text_size(px(9.5))
                            .text_color(ShellDeckColors::text_muted())
                            .child(site),
                    );
                }
            }
            if selected {
                row = row.bg(ShellDeckColors::selected_bg()).child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(9.5))
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("ai.mention.enter").to_string()),
                );
            }
            panel = panel.child(row.on_click(cx.listener(move |this, _, _, cx| {
                this.accept_mention(reference.clone(), cx);
            })));
        }

        Some(
            dismiss_layer(
                "ai-mention-backdrop",
                cx.listener(|this, _e, _window, cx| {
                    this.mention_picker = None;
                    cx.notify();
                }),
            )
            .child(panel)
            .into_any_element(),
        )
    }

    /// One-line footer state: how many attachments ride along, and whether any
    /// of them cannot be delivered by the current backend.
    pub(super) fn attachment_notice(&self) -> Option<String> {
        if let Some(error) = self.attachment_error.clone() {
            return Some(error);
        }
        if self
            .attachments
            .iter()
            .any(|staged| staged.attachment.is_image())
            && !self.backend_takes_images()
        {
            return Some(
                t!(
                    "ai.attachment.error.backend",
                    backend = self.backend.display_name()
                )
                .to_string(),
            );
        }
        None
    }

    /// A staged image the backend cannot carry blocks the send. Failing here is
    /// the point: a turn that quietly drops the screenshot the question is
    /// about produces a confident answer to a question nobody asked.
    pub(super) fn composer_blocked_by_attachments(&self) -> bool {
        validate_attachments(self.backend, &self.staged_attachments()).is_err()
    }
}

/// Where a composer panel may paint.
///
/// Both panels belong to the composer, but they are mounted at the view root
/// (they need a full-surface dismiss layer). So they have to be told which
/// column the composer occupies: in the Sheet the history column eats the left
/// 240px when it is open, and in the Dock the activity rail eats the right
/// 56px. Anchoring them to the window instead is how a dropdown ends up
/// covering the conversation list it has nothing to do with.
fn composer_panel_bounds(host: super::AiHost, history_open: bool) -> (f32, f32) {
    match host {
        super::AiHost::Sheet if history_open => (252.0, 16.0),
        super::AiHost::Sheet => (12.0, 16.0),
        // 56px rail + the same 12px gutter the composer uses.
        super::AiHost::Dock => (12.0, 68.0),
    }
}

fn clamp_boundary(value: &str, offset: usize) -> usize {
    let mut offset = offset.min(value.len());
    while offset > 0 && !value.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn chip_shell(id: impl Into<gpui::ElementId>) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id.into())
        .flex()
        .items_center()
        .gap(px(5.0))
        .max_w_full()
        .min_w(px(0.0))
        .h(px(21.0))
        .pl(px(7.0))
        .pr(px(3.0))
        .rounded(px(6.0))
        .bg(ShellDeckColors::bg_surface())
        .text_size(px(10.5))
        .text_color(ShellDeckColors::text_muted())
}

fn chip_remove(
    id: impl Into<gpui::ElementId>,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    div()
        .id(id.into())
        .flex()
        .items_center()
        .justify_center()
        .flex_shrink_0()
        .size(px(16.0))
        .rounded(px(4.0))
        .cursor_pointer()
        .hover(|style| style.bg(ShellDeckColors::hover_bg()))
        .child(
            gpui::svg()
                .path(lucide_path("x"))
                .size(px(10.0))
                .text_color(ShellDeckColors::text_muted()),
        )
        .on_click(move |event, window, cx| {
            // The chip itself opens the preview; without this the remove
            // control would do both.
            cx.stop_propagation();
            on_click(event, window, cx);
        })
        .into_any_element()
}

/// Same shell as the composer's provider menu, so the three footer panels read
/// as one family (`.agents/ui-components.md` § Harmonization).
fn floating_panel(id: &'static str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .absolute()
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
                offset: point(gpui_px(0.0), gpui_px(4.0)),
                blur_radius: gpui_px(20.0),
                spread_radius: gpui_px(0.0),
                inset: false,
            }]
            .into(),
        )
        .on_mouse_down(MouseButton::Left, |_e, _window, cx: &mut gpui::App| {
            cx.stop_propagation();
        })
}

fn dismiss_layer(
    id: &'static str,
    on_dismiss: impl Fn(&gpui::MouseDownEvent, &mut Window, &mut gpui::App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .occlude()
        .on_mouse_down(MouseButton::Left, on_dismiss)
}

fn menu_row(
    id: &'static str,
    icon: &'static str,
    label: String,
    hint: Option<String>,
    enabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let tint = if enabled {
        ShellDeckColors::text_primary()
    } else {
        ShellDeckColors::text_muted()
    };
    let mut label_column = div().flex().flex_col().flex_1().min_w(px(0.0)).child(
        div()
            .truncate()
            .text_size(px(12.0))
            .text_color(tint)
            .child(SharedString::from(label)),
    );
    if let Some(hint) = hint {
        label_column = label_column.child(
            div()
                .text_size(px(10.0))
                .text_color(ShellDeckColors::text_muted())
                .child(SharedString::from(hint)),
        );
    }
    let mut row = div()
        .id(id)
        .flex()
        .items_center()
        .gap(px(8.0))
        .px(px(9.0))
        .py(px(7.0))
        .rounded(px(7.0))
        .child(lucide_icon(icon, 13.0, tint))
        .child(label_column);
    if enabled {
        row = row
            .cursor_pointer()
            .hover(|style| style.bg(ShellDeckColors::hover_bg()))
            .on_click(on_click);
    } else {
        row = row.opacity(0.6);
    }
    row.into_any_element()
}

/// Chip labels for attachment kinds, used by the footnote counter.
pub(super) fn attachment_summary(attachments: &[AiAttachment]) -> Option<String> {
    if attachments.is_empty() {
        return None;
    }
    let images = attachments
        .iter()
        .filter(|attachment| attachment.kind == AiAttachmentKind::Image)
        .count();
    let texts = attachments.len() - images;
    // Only the halves that exist: "0 image(s), 1 fichier(s) texte" reads like a
    // form field, not like a sentence about what is attached.
    let mut parts: Vec<String> = Vec::new();
    if images > 0 {
        parts.push(t!("ai.attachment.summary.images", count = images.to_string()).to_string());
    }
    if texts > 0 {
        parts.push(t!("ai.attachment.summary.texts", count = texts.to_string()).to_string());
    }
    Some(parts.join(", "))
}
