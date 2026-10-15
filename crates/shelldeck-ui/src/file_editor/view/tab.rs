use super::*;

// ---------------------------------------------------------------------------
// TabContent
// ---------------------------------------------------------------------------

pub struct PdfInfo {
    pub page_count: usize,
    pub file_size: u64,
    pub title: Option<String>,
    pub author: Option<String>,
    pub creator: Option<String>,
}

#[cfg(test)]
mod external_tests {
    use super::*;

    fn document() -> EditorTab {
        EditorTab::from_external(ExternalEditorDocument {
            document_id: "ssh:target:/workspace/main.rs".into(),
            filename: "main.rs".into(),
            display_path: "/workspace/main.rs".into(),
            provenance: "SSH target".into(),
            text: "fn main() {}\n".into(),
            read_only: false,
        })
    }

    fn pending(tab: &mut EditorTab, close_after_save: bool) -> Uuid {
        let request_id = Uuid::new_v4();
        let text = tab.buffer().unwrap().text();
        tab.external.as_mut().unwrap().pending_save = Some(PendingExternalSave {
            request_id,
            text,
            close_after_save,
        });
        request_id
    }

    // SDTEST-2496 · SDUC-475
    #[::core::prelude::v1::test]
    fn sdtest_2496_external_save_preserves_edits_made_during_write_and_close() {
        let mut tab = document();
        assert!(
            tab.path.is_none(),
            "a remote display path must never become a local save target"
        );
        assert_eq!(tab.language(), Some(EditorLanguage::Rust));
        tab.buffer_mut().unwrap().insert_str("// before\n");
        let request_id = pending(&mut tab, true);
        tab.buffer_mut().unwrap().insert_str("// during\n");
        let newest = tab.buffer().unwrap().text();
        assert!(!tab.complete_external_save(request_id, Ok(())));
        assert!(tab.is_dirty());
        assert_eq!(tab.buffer().unwrap().text(), newest);
        let retry = pending(&mut tab, true);
        assert!(tab.complete_external_save(retry, Ok(())));
        assert!(!tab.is_dirty());
    }

    // SDTEST-2497 · SDUC-475
    #[::core::prelude::v1::test]
    fn sdtest_2497_external_save_rejects_stale_completion_and_retains_failed_text() {
        let mut tab = document();
        tab.buffer_mut().unwrap().insert_str("// edit\n");
        let request_id = pending(&mut tab, true);
        let text = tab.buffer().unwrap().text();
        assert!(!tab.complete_external_save(Uuid::new_v4(), Ok(())));
        assert!(tab.external.as_ref().unwrap().pending_save.is_some());
        assert!(tab.is_dirty());
        assert!(!tab.complete_external_save(request_id, Err("remote changed".into())));
        assert!(tab.is_dirty());
        assert_eq!(tab.buffer().unwrap().text(), text);
        assert_eq!(
            tab.external.as_ref().unwrap().save_error.as_deref(),
            Some("remote changed")
        );
        let retry = pending(&mut tab, false);
        assert!(!tab.complete_external_save(request_id, Ok(())));
        assert!(tab.external.as_ref().unwrap().pending_save.is_some());
        assert!(!tab.complete_external_save(retry, Ok(())));
        assert!(!tab.is_dirty());
        assert!(tab.external.as_ref().unwrap().save_error.is_none());
    }
}

pub struct BinaryInfo {
    pub file_size: u64,
}

/// A document whose storage is owned by the host surface. Its display path is
/// never interpreted as a local filesystem path, including for local projects.
pub struct ExternalEditorDocument {
    pub document_id: String,
    pub filename: String,
    pub display_path: String,
    pub provenance: String,
    pub text: String,
    pub read_only: bool,
}

pub(super) struct ExternalDocumentState {
    pub document_id: String,
    pub display_path: String,
    pub provenance: String,
    pub pending_save: Option<PendingExternalSave>,
    pub save_error: Option<String>,
}

pub(super) struct PendingExternalSave {
    pub request_id: Uuid,
    pub text: String,
    pub close_after_save: bool,
}

// `Text` is the common, hot-path variant and is matched in many places by its
// fields; boxing it to equalize variant size would add indirection to every
// edit/render access for no real memory win (open tabs are few).
#[allow(clippy::large_enum_variant)]
pub enum TabContent {
    Text {
        buffer: RopeBuffer,
        highlighter: SyntaxHighlighter,
        language: EditorLanguage,
    },
    Image {
        image_path: PathBuf,
    },
    Pdf {
        info: PdfInfo,
    },
    Binary {
        info: BinaryInfo,
    },
}

// ---------------------------------------------------------------------------
// EditorTab
// ---------------------------------------------------------------------------
pub struct EditorTab {
    pub id: Uuid,
    pub path: Option<PathBuf>,
    pub filename: String,
    pub content: TabContent,
    pub(super) external: Option<ExternalDocumentState>,
    pub scroll_offset: f32,
    /// Horizontal scroll offset in *visual columns* (monospace cells). Floats
    /// so trackpad / pixel-precise scrolls accumulate without stair-stepping.
    /// Clamped to `[0.0, max_col - visible_cols + margin]` on each scroll or
    /// cursor movement.
    pub h_scroll_offset: f32,
}

impl EditorTab {
    pub fn new_empty() -> Self {
        let buffer = RopeBuffer::new("");
        let language = EditorLanguage::PlainText;
        let highlighter = SyntaxHighlighter::new(language);
        Self {
            id: Uuid::new_v4(),
            path: None,
            filename: "untitled".to_string(),
            content: TabContent::Text {
                buffer,
                highlighter,
                language,
            },
            external: None,
            scroll_offset: 0.0,
            h_scroll_offset: 0.0,
        }
    }

    pub fn from_file(path: PathBuf, content: &str) -> Self {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        let language = EditorLanguage::from_filename(&filename);
        let buffer = RopeBuffer::new(content);
        let mut highlighter = SyntaxHighlighter::new(language);
        highlighter.parse_full(buffer.rope());
        Self {
            id: Uuid::new_v4(),
            path: Some(path),
            filename,
            content: TabContent::Text {
                buffer,
                highlighter,
                language,
            },
            external: None,
            scroll_offset: 0.0,
            h_scroll_offset: 0.0,
        }
    }

    pub fn from_image(path: PathBuf) -> Self {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        Self {
            id: Uuid::new_v4(),
            path: Some(path.clone()),
            filename,
            content: TabContent::Image { image_path: path },
            external: None,
            scroll_offset: 0.0,
            h_scroll_offset: 0.0,
        }
    }

    pub fn from_pdf(path: PathBuf, info: PdfInfo) -> Self {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        Self {
            id: Uuid::new_v4(),
            path: Some(path),
            filename,
            content: TabContent::Pdf { info },
            external: None,
            scroll_offset: 0.0,
            h_scroll_offset: 0.0,
        }
    }

    pub fn from_binary(path: PathBuf, info: BinaryInfo) -> Self {
        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();
        Self {
            id: Uuid::new_v4(),
            path: Some(path),
            filename,
            content: TabContent::Binary { info },
            external: None,
            scroll_offset: 0.0,
            h_scroll_offset: 0.0,
        }
    }

    pub fn is_text(&self) -> bool {
        matches!(self.content, TabContent::Text { .. })
    }

    pub fn from_external(document: ExternalEditorDocument) -> Self {
        let language = EditorLanguage::from_filename(&document.filename);
        let mut buffer = RopeBuffer::new(&document.text);
        buffer.set_read_only(document.read_only);
        let mut highlighter = SyntaxHighlighter::new(language);
        highlighter.parse_full(buffer.rope());
        Self {
            id: Uuid::new_v4(),
            path: None,
            filename: document.filename,
            content: TabContent::Text {
                buffer,
                highlighter,
                language,
            },
            external: Some(ExternalDocumentState {
                document_id: document.document_id,
                display_path: document.display_path,
                provenance: document.provenance,
                pending_save: None,
                save_error: None,
            }),
            scroll_offset: 0.0,
            h_scroll_offset: 0.0,
        }
    }

    pub fn external_document_id(&self) -> Option<&str> {
        self.external
            .as_ref()
            .map(|document| document.document_id.as_str())
    }

    pub fn is_read_only(&self) -> bool {
        self.buffer().is_some_and(RopeBuffer::is_read_only)
    }

    /// A stale completion cannot mark a reopened document or a different save
    /// clean. Edits made while the host writes remain dirty and keep the tab.
    pub(super) fn complete_external_save(
        &mut self,
        request_id: Uuid,
        result: Result<(), String>,
    ) -> bool {
        let Some(document) = self.external.as_mut() else {
            return false;
        };
        if document
            .pending_save
            .as_ref()
            .is_none_or(|save| save.request_id != request_id)
        {
            return false;
        }
        let save = document.pending_save.take().unwrap();
        match result {
            Ok(()) => {
                document.save_error = None;
                if let TabContent::Text { buffer, .. } = &mut self.content {
                    let unchanged = buffer.text() == save.text;
                    buffer.set_dirty(!unchanged);
                    unchanged && save.close_after_save
                } else {
                    false
                }
            }
            Err(error) => {
                document.save_error = Some(error);
                false
            }
        }
    }

    pub fn is_dirty(&self) -> bool {
        match &self.content {
            TabContent::Text { buffer, .. } => buffer.is_dirty(),
            _ => false,
        }
    }

    pub fn buffer(&self) -> Option<&RopeBuffer> {
        match &self.content {
            TabContent::Text { buffer, .. } => Some(buffer),
            _ => None,
        }
    }

    pub fn buffer_mut(&mut self) -> Option<&mut RopeBuffer> {
        match &mut self.content {
            TabContent::Text { buffer, .. } => Some(buffer),
            _ => None,
        }
    }

    pub fn highlighter_mut(&mut self) -> Option<&mut SyntaxHighlighter> {
        match &mut self.content {
            TabContent::Text { highlighter, .. } => Some(highlighter),
            _ => None,
        }
    }

    pub fn language(&self) -> Option<EditorLanguage> {
        match &self.content {
            TabContent::Text { language, .. } => Some(*language),
            _ => None,
        }
    }

    /// Returns mutable references to buffer and highlighter.
    /// Panics if not a text tab — only call after an `is_text()` guard.
    pub fn text_parts_mut(&mut self) -> (&mut RopeBuffer, &mut SyntaxHighlighter) {
        match &mut self.content {
            TabContent::Text {
                buffer,
                highlighter,
                ..
            } => (buffer, highlighter),
            _ => panic!("text_parts_mut called on non-text tab"),
        }
    }

    pub fn content_type_name(&self) -> &str {
        match &self.content {
            TabContent::Text { language, .. } => language.display_name(),
            TabContent::Image { .. } => "Image",
            TabContent::Pdf { .. } => "PDF",
            TabContent::Binary { .. } => "Binary",
        }
    }

    pub(super) fn display_name(&self) -> String {
        let dirty = if self.is_dirty() { " *" } else { "" };
        format!("{}{}", self.filename, dirty)
    }
}
