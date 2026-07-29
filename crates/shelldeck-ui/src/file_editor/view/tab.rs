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

pub struct BinaryInfo {
    pub file_size: u64,
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
            scroll_offset: 0.0,
            h_scroll_offset: 0.0,
        }
    }

    pub fn is_text(&self) -> bool {
        matches!(self.content, TabContent::Text { .. })
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
