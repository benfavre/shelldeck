//! Local image intake for hosted requests. Drafts stay in memory until the
//! request/comment is submitted; Manage-scoped tickets then upload them to
//! Inklura Share.

use gpui::{Image, ImageFormat};
use shelldeck_core::config::issues::{IssueAttachmentUpload, ISSUE_ATTACHMENT_MAX_BYTES};
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use crate::t;

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
        attempts.iter().any(|(program, args)| {
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
