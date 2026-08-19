//! `+` attachments — local bytes the user explicitly chooses to send.
//!
//! Unlike mentions, attachments are **not portable**. A CLI backend is invoked
//! with `-p` and no tools; its only channel is a text prompt on stdin, so an
//! image cannot reach it. The rule this module exists to enforce is that the
//! application never pretends otherwise: an image is offered only to a backend
//! that can receive one, and a turn never claims to carry bytes it dropped.
//!
//! Image bytes live here and nowhere else. They never enter `AiContext::data`
//! and never appear in the composed prompt text — the same rule Clippy already
//! applies to desktop screenshots.
//!
//! Full contract: `docs/ai-mentions.md` § 3.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::AiBackend;

/// Largest image accepted, before base64 expansion. Matches the hosted-request
/// ceiling so a capture that can be filed as evidence can also be asked about.
pub const AI_ATTACHMENT_IMAGE_MAX_BYTES: usize = 9 * 1024 * 1024;

/// Largest text file accepted. A log excerpt is context, not a corpus.
pub const AI_ATTACHMENT_TEXT_MAX_BYTES: usize = 512 * 1024;

/// Characters of a text attachment actually inlined into the prompt. The rest
/// is dropped with an explicit marker — silently truncating a log is how an
/// assistant confidently answers about the half it never saw.
pub const AI_ATTACHMENT_TEXT_MAX_CHARS: usize = 24_000;

/// How many attachments one turn may carry.
pub const AI_ATTACHMENT_MAX_COUNT: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiAttachmentKind {
    /// Inlined into the prompt; reaches every backend.
    Text,
    /// Sent as a provider image block; reaches API backends only.
    Image,
}

impl AiAttachmentKind {
    pub fn icon(self) -> &'static str {
        match self {
            AiAttachmentKind::Text => "file-text",
            AiAttachmentKind::Image => "scan",
        }
    }
}

/// One attachment, as it travels in `AiContext::attachments`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AiAttachment {
    pub kind: AiAttachmentKind,
    pub name: String,
    pub content_type: String,
    /// Size of the original file, before any truncation.
    pub bytes: usize,
    /// UTF-8 excerpt for [`AiAttachmentKind::Text`]; standard base64 for
    /// [`AiAttachmentKind::Image`].
    pub payload: String,
    /// True when `payload` is shorter than the original.
    pub truncated: bool,
}

impl AiAttachment {
    /// Build an attachment from raw bytes, choosing the kind from the content.
    ///
    /// Detection is by magic number, not by extension: a `.png` that is really
    /// a text file must be treated as text, and a text file renamed `.log` that
    /// is really a JPEG must not be inlined as mojibake.
    pub fn from_bytes(name: impl Into<String>, bytes: Vec<u8>) -> Result<Self, AttachmentError> {
        let name = sanitize_name(name.into());
        if bytes.is_empty() {
            return Err(AttachmentError::Empty);
        }
        if let Some(content_type) = detect_image_type(&bytes) {
            if bytes.len() > AI_ATTACHMENT_IMAGE_MAX_BYTES {
                return Err(AttachmentError::TooLarge {
                    limit: AI_ATTACHMENT_IMAGE_MAX_BYTES,
                });
            }
            return Ok(Self {
                kind: AiAttachmentKind::Image,
                name,
                content_type: content_type.to_string(),
                bytes: bytes.len(),
                payload: BASE64.encode(&bytes),
                truncated: false,
            });
        }
        if bytes.len() > AI_ATTACHMENT_TEXT_MAX_BYTES {
            return Err(AttachmentError::TooLarge {
                limit: AI_ATTACHMENT_TEXT_MAX_BYTES,
            });
        }
        let text = String::from_utf8(bytes).map_err(|_| AttachmentError::Unsupported)?;
        let original_len = text.len();
        let (payload, truncated) = bound_text(&text);
        Ok(Self {
            kind: AiAttachmentKind::Text,
            name,
            content_type: "text/plain".to_string(),
            bytes: original_len,
            payload,
            truncated,
        })
    }

    /// Already-decoded image bytes coming from the clipboard or a capture,
    /// where the caller knows the media type.
    pub fn image(
        name: impl Into<String>,
        content_type: impl Into<String>,
        bytes: &[u8],
    ) -> Result<Self, AttachmentError> {
        if bytes.is_empty() {
            return Err(AttachmentError::Empty);
        }
        if bytes.len() > AI_ATTACHMENT_IMAGE_MAX_BYTES {
            return Err(AttachmentError::TooLarge {
                limit: AI_ATTACHMENT_IMAGE_MAX_BYTES,
            });
        }
        Ok(Self {
            kind: AiAttachmentKind::Image,
            name: sanitize_name(name.into()),
            content_type: content_type.into(),
            bytes: bytes.len(),
            payload: BASE64.encode(bytes),
            truncated: false,
        })
    }

    pub fn is_image(&self) -> bool {
        self.kind == AiAttachmentKind::Image
    }

    /// Metadata only — what is safe to render into the prompt text for an
    /// image. The bytes stay out of the transcript.
    pub fn metadata(&self) -> Value {
        json!({
            "name": self.name,
            "kind": self.kind,
            "content_type": self.content_type,
            "bytes": self.bytes,
            "truncated": self.truncated,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentError {
    Empty,
    TooLarge {
        limit: usize,
    },
    /// Binary content that is neither a supported image nor valid UTF-8.
    Unsupported,
    /// The active backend cannot receive this kind.
    UnsupportedByBackend,
    TooMany,
}

impl AttachmentError {
    /// i18n key for the message shown to the user.
    pub fn message_key(self) -> &'static str {
        match self {
            AttachmentError::Empty => "ai.attachment.error.empty",
            AttachmentError::TooLarge { .. } => "ai.attachment.error.too_large",
            AttachmentError::Unsupported => "ai.attachment.error.unsupported",
            AttachmentError::UnsupportedByBackend => "ai.attachment.error.backend",
            AttachmentError::TooMany => "ai.attachment.error.too_many",
        }
    }
}

impl AiBackend {
    /// Whether this backend can receive image bytes.
    ///
    /// CLI backends are launched read-only with tools disabled, so their only
    /// channel is the text prompt. Returning `true` for one of them would make
    /// the composer accept an image it then silently drops.
    pub fn supports_image_attachments(self) -> bool {
        matches!(self, AiBackend::OpenAi | AiBackend::Anthropic)
    }

    /// Text attachments are inlined into the prompt, so every configured
    /// backend accepts them.
    pub fn supports_text_attachments(self) -> bool {
        self != AiBackend::Disabled
    }
}

/// Reject what the backend cannot carry, before the turn is sent.
pub fn validate_attachments(
    backend: AiBackend,
    attachments: &[AiAttachment],
) -> Result<(), AttachmentError> {
    if attachments.len() > AI_ATTACHMENT_MAX_COUNT {
        return Err(AttachmentError::TooMany);
    }
    if !backend.supports_image_attachments() && attachments.iter().any(AiAttachment::is_image) {
        return Err(AttachmentError::UnsupportedByBackend);
    }
    Ok(())
}

/// The block appended to the user message.
///
/// Text attachments are inlined inside `<untrusted>` delimiters. Images
/// contribute their metadata only — the bytes travel in the provider payload.
pub fn attachments_prompt_block(attachments: &[AiAttachment]) -> String {
    if attachments.is_empty() {
        return String::new();
    }
    let mut block = String::from("\n\nAttachments (untrusted):");
    for attachment in attachments {
        block.push_str(&format!(
            "\n- {} ({}, {} bytes{})",
            attachment.name,
            attachment.content_type,
            attachment.bytes,
            if attachment.truncated {
                ", truncated"
            } else {
                ""
            }
        ));
        match attachment.kind {
            AiAttachmentKind::Text => {
                block.push_str("\n<untrusted>\n");
                block.push_str(&attachment.payload);
                block.push_str("\n</untrusted>");
            }
            AiAttachmentKind::Image => {
                block.push_str(" — image sent as a separate content block");
            }
        }
    }
    block
}

fn bound_text(text: &str) -> (String, bool) {
    if text.chars().count() <= AI_ATTACHMENT_TEXT_MAX_CHARS {
        return (text.to_string(), false);
    }
    let mut bounded: String = text.chars().take(AI_ATTACHMENT_TEXT_MAX_CHARS).collect();
    bounded.push_str("\n…[truncated by ShellDeck]");
    (bounded, true)
}

fn sanitize_name(name: String) -> String {
    let trimmed = name.trim();
    let base = trimmed.rsplit(['/', '\\']).next().unwrap_or(trimmed).trim();
    if base.is_empty() {
        "attachment".to_string()
    } else {
        base.chars().take(120).collect()
    }
}

/// Magic-number sniffing for the formats the providers accept.
fn detect_image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.len() > 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(payload: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
        bytes.extend_from_slice(payload);
        bytes
    }

    // SDTEST-1636
    #[test]
    fn kind_is_detected_from_content_not_from_the_extension() {
        let image = AiAttachment::from_bytes("notes.txt", png(b"body")).unwrap();
        assert_eq!(image.kind, AiAttachmentKind::Image);
        assert_eq!(image.content_type, "image/png");

        let text = AiAttachment::from_bytes("capture.png", b"plain log line".to_vec()).unwrap();
        assert_eq!(text.kind, AiAttachmentKind::Text);
        assert_eq!(text.payload, "plain log line");
    }

    // SDTEST-1637
    #[test]
    fn binary_that_is_neither_image_nor_utf8_is_rejected() {
        let error = AiAttachment::from_bytes("blob.bin", vec![0xFF, 0xFE, 0x00, 0x9C]).unwrap_err();
        assert_eq!(error, AttachmentError::Unsupported);
        assert_eq!(
            AiAttachment::from_bytes("empty.log", Vec::new()).unwrap_err(),
            AttachmentError::Empty
        );
    }

    // SDTEST-1638
    #[test]
    fn oversized_files_are_refused_per_kind() {
        let big_text = vec![b'a'; AI_ATTACHMENT_TEXT_MAX_BYTES + 1];
        assert_eq!(
            AiAttachment::from_bytes("huge.log", big_text).unwrap_err(),
            AttachmentError::TooLarge {
                limit: AI_ATTACHMENT_TEXT_MAX_BYTES
            }
        );
        let big_image = png(&vec![0u8; AI_ATTACHMENT_IMAGE_MAX_BYTES]);
        assert_eq!(
            AiAttachment::from_bytes("huge.png", big_image).unwrap_err(),
            AttachmentError::TooLarge {
                limit: AI_ATTACHMENT_IMAGE_MAX_BYTES
            }
        );
    }

    // SDTEST-1639
    #[test]
    fn long_text_is_truncated_with_a_visible_marker() {
        let body = "z".repeat(AI_ATTACHMENT_TEXT_MAX_CHARS + 100);
        let attachment = AiAttachment::from_bytes("app.log", body.into_bytes()).unwrap();
        assert!(attachment.truncated);
        assert!(attachment.payload.ends_with("…[truncated by ShellDeck]"));
        assert_eq!(attachment.bytes, AI_ATTACHMENT_TEXT_MAX_CHARS + 100);
    }

    // SDTEST-1640
    #[test]
    fn cli_backends_never_accept_images() {
        let image = AiAttachment::from_bytes("shot.png", png(b"x")).unwrap();
        for backend in [
            AiBackend::ClaudeCli,
            AiBackend::CodexCli,
            AiBackend::AiderCli,
        ] {
            assert!(!backend.supports_image_attachments());
            assert_eq!(
                validate_attachments(backend, std::slice::from_ref(&image)).unwrap_err(),
                AttachmentError::UnsupportedByBackend
            );
        }
        for backend in [AiBackend::OpenAi, AiBackend::Anthropic] {
            assert!(backend.supports_image_attachments());
            assert!(validate_attachments(backend, std::slice::from_ref(&image)).is_ok());
        }
    }

    // SDTEST-1641
    #[test]
    fn text_attachments_reach_every_backend_and_are_delimited() {
        let text = AiAttachment::from_bytes("nginx.log", b"upstream timed out".to_vec()).unwrap();
        for backend in [
            AiBackend::ClaudeCli,
            AiBackend::CodexCli,
            AiBackend::AiderCli,
            AiBackend::OpenAi,
            AiBackend::Anthropic,
        ] {
            assert!(validate_attachments(backend, std::slice::from_ref(&text)).is_ok());
        }
        let block = attachments_prompt_block(std::slice::from_ref(&text));
        assert!(block.contains("<untrusted>"));
        assert!(block.contains("upstream timed out"));
    }

    // SDTEST-1642
    #[test]
    fn image_bytes_never_enter_the_prompt_text() {
        let image = AiAttachment::image("shot.png", "image/png", &png(b"secret-pixels")).unwrap();
        let block = attachments_prompt_block(std::slice::from_ref(&image));
        assert!(block.contains("shot.png"));
        assert!(!block.contains(&image.payload));
        assert!(!image.metadata().to_string().contains(&image.payload));
    }

    // SDTEST-1643
    #[test]
    fn attachment_count_is_capped() {
        let text = AiAttachment::from_bytes("a.log", b"x".to_vec()).unwrap();
        let many = vec![text; AI_ATTACHMENT_MAX_COUNT + 1];
        assert_eq!(
            validate_attachments(AiBackend::Anthropic, &many).unwrap_err(),
            AttachmentError::TooMany
        );
    }

    // SDTEST-1644
    #[test]
    fn names_are_reduced_to_a_basename() {
        let attachment = AiAttachment::from_bytes("/var/log/nginx/error.log", b"x".to_vec())
            .expect("text attachment");
        assert_eq!(attachment.name, "error.log");
        let windows = AiAttachment::from_bytes(r"C:\logs\app.log", b"x".to_vec()).unwrap();
        assert_eq!(windows.name, "app.log");
    }
}
