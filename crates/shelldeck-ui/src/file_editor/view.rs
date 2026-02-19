use std::path::PathBuf;
use std::sync::Arc;

use gpui::prelude::*;
use gpui::*;
use uuid::Uuid;

use super::buffer::RopeBuffer;
use super::file_browser::FileBrowserPanel;
use super::highlighter::{HighlightSpan, SyntaxHighlighter};
use super::{EditorLanguage, FileKind};
use crate::glyph_cache::GlyphCache;
use crate::theme::ShellDeckColors;

// ---------------------------------------------------------------------------
// Layout constants
// ---------------------------------------------------------------------------
const TAB_BAR_HEIGHT: f32 = 36.0;
const SEARCH_BAR_HEIGHT: f32 = 32.0;
const GOTO_LINE_BAR_HEIGHT: f32 = 32.0;
const STATUS_BAR_HEIGHT: f32 = 22.0;
const REPLACE_BAR_HEIGHT: f32 = 32.0;
const DRAG_HANDLE_WIDTH: f32 = 4.0;
const SCROLLBAR_WIDTH: f32 = 8.0;

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------
actions!(
    file_editor,
    [
        OpenFileEditor,
        OpenFile,
        SaveFile,
        CloseEditorTab,
        EditorUndo,
        EditorRedo,
        EditorSelectAll,
        EditorDuplicateLine,
        EditorDeleteLine,
        EditorToggleSearch,
        EditorGotoLine,
        ToggleFileBrowser,
    ]
);

// ---------------------------------------------------------------------------
// Context menu
// ---------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuAction {
    Cut,
    Copy,
    Paste,
    SelectAll,
    Undo,
    Redo,
    ToggleComment,
}

struct ContextMenuItem {
    label: &'static str,
    shortcut: &'static str,
    action: ContextMenuAction,
}

const CONTEXT_MENU_ITEMS: &[ContextMenuItem] = &[
    ContextMenuItem { label: "Undo", shortcut: "Ctrl+Z", action: ContextMenuAction::Undo },
    ContextMenuItem { label: "Redo", shortcut: "Ctrl+Y", action: ContextMenuAction::Redo },
    ContextMenuItem { label: "Cut", shortcut: "Ctrl+X", action: ContextMenuAction::Cut },
    ContextMenuItem { label: "Copy", shortcut: "Ctrl+C", action: ContextMenuAction::Copy },
    ContextMenuItem { label: "Paste", shortcut: "Ctrl+V", action: ContextMenuAction::Paste },
    ContextMenuItem { label: "Select All", shortcut: "Ctrl+A", action: ContextMenuAction::SelectAll },
    ContextMenuItem { label: "Toggle Comment", shortcut: "Ctrl+/", action: ContextMenuAction::ToggleComment },
];

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub enum FileEditorEvent {
    TabsChanged,
}

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

    fn display_name(&self) -> String {
        let dirty = if self.is_dirty() { " *" } else { "" };
        format!("{}{}", self.filename, dirty)
    }
}

// ---------------------------------------------------------------------------
// FileEditorView
// ---------------------------------------------------------------------------
pub struct FileEditorView {
    pub tabs: Vec<EditorTab>,
    pub active_tab_index: usize,
    pub focus_handle: FocusHandle,
    pub(crate) glyph_cache: Option<Arc<GlyphCache>>,
    pub(crate) cursor_blink_on: bool,
    pub(crate) cursor_blink_task: Option<Task<()>>,
    pub(crate) scroll_lines_per_page: usize,
    pub(crate) mouse_selecting: bool,
    // File browser
    pub file_browser: FileBrowserPanel,
    pub file_browser_visible: bool,
    pub(crate) file_browser_width: f32,
    pub(crate) file_browser_resizing: bool,
    // Search
    pub(crate) search_visible: bool,
    pub(crate) search_query: String,
    pub(crate) search_matches: Vec<std::ops::Range<usize>>,
    pub(crate) search_current_idx: Option<usize>,
    pub(crate) search_case_sensitive: bool,
    // Replace
    pub(crate) replace_visible: bool,
    pub(crate) replace_query: String,
    pub(crate) search_focus_replace: bool,
    // Go-to-line
    pub(crate) goto_line_visible: bool,
    pub(crate) goto_line_query: String,
    // Context menu
    pub(crate) context_menu_visible: bool,
    pub(crate) context_menu_position: (f32, f32),
    // Interactive scrollbar
    pub(crate) scrollbar_dragging: bool,
    // Mouse click origin for dead zone (prevents micro-movements from changing cursor)
    pub(crate) mouse_click_origin: Option<(f32, f32)>,
    // Unsaved changes warning
    pub(crate) pending_close_tab: Option<usize>,
    // Cached layout
    pub(crate) font_family: String,
    pub(crate) font_size: f32,
    // Canvas bounds origin (set during prepaint, used by mouse handlers)
    pub(crate) canvas_origin: std::sync::Arc<std::sync::atomic::AtomicU64>,
    // Canvas height in pixels (set during prepaint, used for scroll_lines_per_page)
    pub(crate) canvas_height: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

impl EventEmitter<FileEditorEvent> for FileEditorView {}

impl FileEditorView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            tabs: Vec::new(),
            active_tab_index: 0,
            focus_handle: cx.focus_handle(),
            glyph_cache: None,
            cursor_blink_on: true,
            cursor_blink_task: None,
            scroll_lines_per_page: 30,
            mouse_selecting: false,
            file_browser: FileBrowserPanel::new(),
            file_browser_visible: true,
            file_browser_width: 220.0,
            file_browser_resizing: false,
            search_visible: false,
            search_query: String::new(),
            search_matches: Vec::new(),
            search_current_idx: None,
            search_case_sensitive: false,
            replace_visible: false,
            replace_query: String::new(),
            search_focus_replace: false,
            goto_line_visible: false,
            goto_line_query: String::new(),
            context_menu_visible: false,
            context_menu_position: (0.0, 0.0),
            scrollbar_dragging: false,
            mouse_click_origin: None,
            pending_close_tab: None,
            font_family: "JetBrains Mono".to_string(),
            font_size: 14.0,
            canvas_origin: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
            canvas_height: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
        };
        // Start with one empty tab
        view.tabs.push(EditorTab::new_empty());
        view
    }

    // -----------------------------------------------------------------------
    // Tab management
    // -----------------------------------------------------------------------

    pub fn active_tab(&self) -> Option<&EditorTab> {
        self.tabs.get(self.active_tab_index)
    }

    pub fn active_tab_mut(&mut self) -> Option<&mut EditorTab> {
        self.tabs.get_mut(self.active_tab_index)
    }

    pub fn open_file(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        // Check if already open
        for (i, tab) in self.tabs.iter().enumerate() {
            if tab.path.as_ref() == Some(&path) {
                self.active_tab_index = i;
                cx.notify();
                return;
            }
        }

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");
        let kind = FileKind::from_filename(filename);

        let tab = match kind {
            FileKind::Image => EditorTab::from_image(path),
            FileKind::Pdf => {
                match Self::load_pdf_info(&path) {
                    Some(info) => EditorTab::from_pdf(path, info),
                    None => {
                        // Fallback to binary if PDF parsing fails
                        let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        EditorTab::from_binary(path, BinaryInfo { file_size })
                    }
                }
            }
            FileKind::Binary => {
                let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                EditorTab::from_binary(path, BinaryInfo { file_size })
            }
            FileKind::Text => {
                match std::fs::read_to_string(&path) {
                    Ok(content) => EditorTab::from_file(path, &content),
                    Err(_) => {
                        // UTF-8 decode failed — treat as binary
                        let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                        EditorTab::from_binary(path, BinaryInfo { file_size })
                    }
                }
            }
        };

        // Replace empty untitled tab instead of adding alongside it
        let replace_empty = self.tabs.len() == 1
            && self.tabs[0].path.is_none()
            && !self.tabs[0].is_dirty()
            && self.tabs[0].buffer().map_or(true, |b| b.len_chars() == 0);
        if replace_empty {
            self.tabs[0] = tab;
            self.active_tab_index = 0;
        } else {
            self.tabs.push(tab);
            self.active_tab_index = self.tabs.len() - 1;
        }
        cx.emit(FileEditorEvent::TabsChanged);
        cx.notify();
    }

    pub fn save_file(&mut self, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get_mut(self.active_tab_index) {
            if let (Some(ref path), TabContent::Text { buffer, .. }) =
                (&tab.path, &mut tab.content)
            {
                let content = buffer.text();
                match std::fs::write(path, &content) {
                    Ok(()) => {
                        buffer.set_dirty(false);
                        cx.notify();
                    }
                    Err(e) => {
                        tracing::error!("Failed to save file: {}", e);
                    }
                }
            }
        }
    }

    pub fn close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        // Check for unsaved changes
        if let Some(tab) = self.tabs.get(index) {
            if tab.is_dirty() && self.pending_close_tab.is_none() {
                self.pending_close_tab = Some(index);
                cx.notify();
                return;
            }
        }
        self.force_close_tab(index, cx);
    }

    pub fn force_close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        self.pending_close_tab = None;
        if self.tabs.len() <= 1 {
            self.tabs[0] = EditorTab::new_empty();
            self.active_tab_index = 0;
        } else {
            self.tabs.remove(index);
            if self.active_tab_index >= self.tabs.len() {
                self.active_tab_index = self.tabs.len() - 1;
            }
        }
        cx.emit(FileEditorEvent::TabsChanged);
        cx.notify();
    }

    pub fn save_and_close_tab(&mut self, index: usize, cx: &mut Context<Self>) {
        // Save first (only for text tabs)
        if let Some(tab) = self.tabs.get_mut(index) {
            if let (Some(ref path), TabContent::Text { buffer, .. }) =
                (&tab.path, &mut tab.content)
            {
                let content = buffer.text();
                match std::fs::write(path, &content) {
                    Ok(()) => {
                        buffer.set_dirty(false);
                    }
                    Err(e) => {
                        tracing::error!("Failed to save file: {}", e);
                        self.pending_close_tab = None;
                        cx.notify();
                        return;
                    }
                }
            }
        }
        self.force_close_tab(index, cx);
    }

    // -----------------------------------------------------------------------
    // Glyph cache
    // -----------------------------------------------------------------------

    fn ensure_glyph_cache(&mut self, window: &Window) {
        if self.glyph_cache.is_none() {
            self.glyph_cache = Some(Arc::new(GlyphCache::build(
                window.text_system(),
                &self.font_family,
                self.font_size,
            )));
        }
    }

    // -----------------------------------------------------------------------
    // Cursor blink
    // -----------------------------------------------------------------------

    fn start_cursor_blink(&mut self, cx: &mut Context<Self>) {
        self.cursor_blink_on = true;
        let handle = cx.entity().downgrade();
        self.cursor_blink_task = Some(cx.spawn(async move |_, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(530))
                    .await;
                let Ok(alive) = cx.update(|cx| {
                    if let Some(view) = handle.upgrade() {
                        view.update(cx, |this, cx| {
                            this.cursor_blink_on = !this.cursor_blink_on;
                            cx.notify();
                        });
                        true
                    } else {
                        false
                    }
                }) else {
                    break;
                };
                if !alive {
                    break;
                }
            }
        }));
    }

    pub(crate) fn reset_cursor_blink(&mut self, cx: &mut Context<Self>) {
        self.cursor_blink_on = true;
        self.cursor_blink_task = None; // drop old task before spawning new
        self.start_cursor_blink(cx);
    }

    // -----------------------------------------------------------------------
    // Scroll management
    // -----------------------------------------------------------------------

    pub(crate) fn ensure_cursor_visible(&mut self) {
        let idx = self.active_tab_index;
        let lines_per_page = self.scroll_lines_per_page;
        let tab = match self.tabs.get(idx) {
            Some(t) => t,
            None => return,
        };
        let buffer = match tab.buffer() {
            Some(b) => b,
            None => return,
        };
        let (cursor_line, _) = buffer.cursor_line_col();
        let first_visible = tab.scroll_offset as usize;
        let last_visible = first_visible + lines_per_page.saturating_sub(1);

        if cursor_line < first_visible {
            self.tabs[idx].scroll_offset = cursor_line as f32;
        } else if cursor_line > last_visible {
            self.tabs[idx].scroll_offset = (cursor_line - lines_per_page + 1) as f32;
        }
    }

    pub(crate) fn clamp_scroll(&mut self) {
        let half_page = (self.scroll_lines_per_page / 2) as f32;
        if let Some(tab) = self.active_tab_mut() {
            if let Some(buffer) = tab.buffer() {
                // Allow scrolling past end by half a page
                let max = buffer.len_lines().saturating_sub(1) as f32 + half_page;
                tab.scroll_offset = tab.scroll_offset.clamp(0.0, max);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Search
    // -----------------------------------------------------------------------

    pub(crate) fn perform_search(&mut self) {
        self.search_matches.clear();
        self.search_current_idx = None;

        if self.search_query.is_empty() {
            return;
        }

        if let Some(buffer) = self.active_tab().and_then(|t| t.buffer()) {
            let text = buffer.text();
            let query = &self.search_query;
            let query_char_len = query.chars().count();

            if self.search_case_sensitive {
                // Case-sensitive search
                let mut byte_start = 0;
                while let Some(byte_pos) = text[byte_start..].find(query.as_str()) {
                    let abs_byte = byte_start + byte_pos;
                    let char_start = text[..abs_byte].chars().count();
                    self.search_matches
                        .push(char_start..char_start + query_char_len);
                    byte_start = abs_byte + query.len();
                }
            } else {
                // Case-insensitive search
                let query_lower = query.to_lowercase();
                let text_lower = text.to_lowercase();
                let mut byte_start = 0;
                while let Some(byte_pos) = text_lower[byte_start..].find(&query_lower) {
                    let abs_byte = byte_start + byte_pos;
                    let char_start = text[..abs_byte].chars().count();
                    self.search_matches
                        .push(char_start..char_start + query_char_len);
                    byte_start = abs_byte + query_lower.len();
                }
            }
            if !self.search_matches.is_empty() {
                self.search_current_idx = Some(0);
            }
        }
    }

    pub(crate) fn search_next(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let idx = self
            .search_current_idx
            .map(|i| (i + 1) % self.search_matches.len())
            .unwrap_or(0);
        self.search_current_idx = Some(idx);
        let char_pos = self.search_matches.get(idx).map(|r| r.start);
        if let Some(pos) = char_pos {
            let tab_idx = self.active_tab_index;
            if let Some(tab) = self.tabs.get_mut(tab_idx) {
                if let Some(buffer) = tab.buffer_mut() {
                    let (line, col) = buffer.char_to_line_col(pos);
                    buffer.set_cursor_from_position(line, col, false);
                }
            }
            self.ensure_cursor_visible();
        }
    }

    pub(crate) fn search_prev(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        let match_len = self.search_matches.len();
        let idx = self
            .search_current_idx
            .map(|i| if i == 0 { match_len - 1 } else { i - 1 })
            .unwrap_or(0);
        self.search_current_idx = Some(idx);
        let char_pos = self.search_matches.get(idx).map(|r| r.start);
        if let Some(pos) = char_pos {
            let tab_idx = self.active_tab_index;
            if let Some(tab) = self.tabs.get_mut(tab_idx) {
                if let Some(buffer) = tab.buffer_mut() {
                    let (line, col) = buffer.char_to_line_col(pos);
                    buffer.set_cursor_from_position(line, col, false);
                }
            }
            self.ensure_cursor_visible();
        }
    }

    // -----------------------------------------------------------------------
    // Replace
    // -----------------------------------------------------------------------

    pub(crate) fn replace_next(&mut self, cx: &mut Context<Self>) {
        if self.search_matches.is_empty() {
            return;
        }
        let idx = self.search_current_idx.unwrap_or(0);
        let match_range = match self.search_matches.get(idx) {
            Some(r) => r.clone(),
            None => return,
        };

        let tab_idx = self.active_tab_index;
        if let Some(tab) = self.tabs.get_mut(tab_idx) {
            if let TabContent::Text { buffer, highlighter, .. } = &mut tab.content {
                // Set selection to the match and delete it, then insert replacement
                let (sl, sc) = buffer.char_to_line_col(match_range.start);
                let (el, ec) = buffer.char_to_line_col(match_range.end);
                buffer.set_cursor_from_position(sl, sc, false);
                buffer.set_cursor_from_position(el, ec, true);
                buffer.delete_selection();
                buffer.insert_str(&self.replace_query);
                highlighter.parse_full(buffer.rope());
            }
        }

        // Re-run search to update matches
        self.perform_search();
        // Try to keep the same index (it will point to the next match)
        if !self.search_matches.is_empty() {
            let new_idx = idx.min(self.search_matches.len() - 1);
            self.search_current_idx = Some(new_idx);
        }
        self.ensure_cursor_visible();
        cx.notify();
    }

    pub(crate) fn replace_all(&mut self, cx: &mut Context<Self>) {
        if self.search_matches.is_empty() {
            return;
        }

        let tab_idx = self.active_tab_index;
        let replace_text = self.replace_query.clone();

        if let Some(tab) = self.tabs.get_mut(tab_idx) {
            if let TabContent::Text { buffer, highlighter, .. } = &mut tab.content {
                // Replace from end to start to preserve earlier positions
                let mut matches: Vec<std::ops::Range<usize>> = self.search_matches.clone();
                matches.reverse();

                buffer.flush_transaction();
                for m in &matches {
                    let (sl, sc) = buffer.char_to_line_col(m.start);
                    let (el, ec) = buffer.char_to_line_col(m.end);
                    buffer.set_cursor_from_position(sl, sc, false);
                    buffer.set_cursor_from_position(el, ec, true);
                    buffer.delete_selection();
                    buffer.insert_str(&replace_text);
                }
                highlighter.parse_full(buffer.rope());
            }
        }

        self.perform_search();
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // Rendering: Tab bar
    // -----------------------------------------------------------------------

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let handle = cx.entity().downgrade();

        let mut tab_bar = div()
            .flex()
            .items_center()
            .w_full()
            .h(px(TAB_BAR_HEIGHT))
            .bg(ShellDeckColors::bg_sidebar())
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .px(px(4.0))
            .gap(px(2.0));

        for (i, tab) in self.tabs.iter().enumerate() {
            let is_active = i == self.active_tab_index;
            let tab_id = tab.id;
            let h1 = handle.clone();
            let h2 = handle.clone();

            let mut tab_el = div()
                .id(SharedString::from(format!("editor-tab-{}", i)))
                .flex()
                .items_center()
                .gap(px(4.0))
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(4.0))
                .cursor_pointer()
                .text_size(px(12.0));

            if is_active {
                tab_el = tab_el
                    .bg(ShellDeckColors::bg_primary())
                    .text_color(ShellDeckColors::text_primary());
            } else {
                tab_el = tab_el
                    .text_color(ShellDeckColors::text_muted())
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()));
            }

            let idx = i;
            tab_el = tab_el.on_click(move |_event, _window, cx| {
                if let Some(view) = h1.upgrade() {
                    view.update(cx, |this, cx| {
                        this.active_tab_index = idx;
                        cx.notify();
                    });
                }
            });

            let name = tab.display_name();
            tab_el = tab_el.child(name);

            // Close button
            if self.tabs.len() > 1 || tab.is_dirty() {
                let close_btn = div()
                    .id(SharedString::from(format!("close-tab-{}", i)))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|s| s.text_color(ShellDeckColors::text_primary()))
                    .child("×")
                    .on_click(move |_event, _window, cx| {
                        if let Some(view) = h2.upgrade() {
                            view.update(cx, |this, cx| {
                                // Find tab by id
                                if let Some(pos) =
                                    this.tabs.iter().position(|t| t.id == tab_id)
                                {
                                    this.close_tab(pos, cx);
                                }
                            });
                        }
                    });
                tab_el = tab_el.child(close_btn);
            }

            tab_bar = tab_bar.child(tab_el);
        }

        // Language indicator on the right
        if let Some(tab) = self.active_tab() {
            let spacer = div().flex_grow();
            let lang_label = div()
                .text_size(px(11.0))
                .text_color(ShellDeckColors::text_muted())
                .px(px(8.0))
                .child(tab.content_type_name().to_string());
            tab_bar = tab_bar.child(spacer).child(lang_label);
        }

        tab_bar
    }

    // -----------------------------------------------------------------------
    // Rendering: Search bar
    // -----------------------------------------------------------------------

    fn render_search_bar(&self) -> impl IntoElement {
        let match_count = self.search_matches.len();
        let current = self
            .search_current_idx
            .map(|i| format!("{}/{}", i + 1, match_count))
            .unwrap_or_else(|| format!("0/{}", match_count));

        let search_focused = !self.search_focus_replace;
        let search_border = if search_focused {
            ShellDeckColors::primary()
        } else {
            ShellDeckColors::border()
        };

        let case_label = if self.search_case_sensitive {
            "[Aa]"
        } else {
            "[aa]"
        };
        let case_color = if self.search_case_sensitive {
            ShellDeckColors::primary()
        } else {
            ShellDeckColors::text_muted()
        };

        div()
            .flex()
            .items_center()
            .w_full()
            .h(px(SEARCH_BAR_HEIGHT))
            .bg(ShellDeckColors::bg_surface())
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .px(px(8.0))
            .gap(px(6.0))
            .text_size(px(12.0))
            .child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child("Find:"),
            )
            .child(
                div()
                    .flex_grow()
                    .px(px(4.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .border_1()
                    .border_color(search_border)
                    .bg(ShellDeckColors::bg_primary())
                    .text_color(ShellDeckColors::text_primary())
                    .child(if self.search_query.is_empty() {
                        "Type to search...".to_string()
                    } else {
                        self.search_query.clone()
                    }),
            )
            .child(
                div()
                    .text_color(case_color)
                    .text_size(px(10.0))
                    .child(case_label),
            )
            .child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child(current),
            )
    }

    fn render_replace_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let h_next = cx.entity().downgrade();
        let h_all = cx.entity().downgrade();
        let replace_focused = self.search_focus_replace;
        let replace_border = if replace_focused {
            ShellDeckColors::primary()
        } else {
            ShellDeckColors::border()
        };

        div()
            .flex()
            .items_center()
            .w_full()
            .h(px(REPLACE_BAR_HEIGHT))
            .bg(ShellDeckColors::bg_surface())
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .px(px(8.0))
            .gap(px(6.0))
            .text_size(px(12.0))
            .child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child("Replace:"),
            )
            .child(
                div()
                    .flex_grow()
                    .px(px(4.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .border_1()
                    .border_color(replace_border)
                    .bg(ShellDeckColors::bg_primary())
                    .text_color(ShellDeckColors::text_primary())
                    .child(if self.replace_query.is_empty() {
                        "Replace with...".to_string()
                    } else {
                        self.replace_query.clone()
                    }),
            )
            .child(
                div()
                    .id("replace-next-btn")
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()).text_color(ShellDeckColors::text_primary()))
                    .child("Replace")
                    .on_click(move |_event, _window, cx| {
                        if let Some(view) = h_next.upgrade() {
                            view.update(cx, |this, cx| {
                                this.replace_next(cx);
                            });
                        }
                    }),
            )
            .child(
                div()
                    .id("replace-all-btn")
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .text_size(px(10.0))
                    .text_color(ShellDeckColors::text_muted())
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()).text_color(ShellDeckColors::text_primary()))
                    .child("All")
                    .on_click(move |_event, _window, cx| {
                        if let Some(view) = h_all.upgrade() {
                            view.update(cx, |this, cx| {
                                this.replace_all(cx);
                            });
                        }
                    }),
            )
    }

    // -----------------------------------------------------------------------
    // Rendering: Go-to-line bar
    // -----------------------------------------------------------------------

    fn render_goto_line_bar(&self) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .w_full()
            .h(px(GOTO_LINE_BAR_HEIGHT))
            .bg(ShellDeckColors::bg_surface())
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .px(px(8.0))
            .gap(px(6.0))
            .text_size(px(12.0))
            .child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child("Go to Line:"),
            )
            .child(
                div()
                    .flex_grow()
                    .px(px(4.0))
                    .py(px(2.0))
                    .rounded(px(3.0))
                    .bg(ShellDeckColors::bg_primary())
                    .text_color(ShellDeckColors::text_primary())
                    .child(if self.goto_line_query.is_empty() {
                        "Line number...".to_string()
                    } else {
                        self.goto_line_query.clone()
                    }),
            )
    }

    // -----------------------------------------------------------------------
    // Rendering: Canvas (the main editor surface)
    // -----------------------------------------------------------------------

    fn render_editor_canvas(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Stateful<Div> {
        self.ensure_glyph_cache(window);
        let cache = match self.glyph_cache.as_ref() {
            Some(c) => c.clone(),
            None => return div().id("editor-canvas-empty"),
        };

        let tab_idx = self.active_tab_index;
        let scroll_lines_per_page = self.scroll_lines_per_page;
        let cursor_blink_on = self.cursor_blink_on;
        let has_focus = self.focus_handle.is_focused(window);

        let tab = match self.tabs.get_mut(tab_idx) {
            Some(t) => t,
            None => return div().id("editor-canvas-empty2"),
        };

        let (buffer, highlighter) = match &mut tab.content {
            TabContent::Text {
                buffer,
                highlighter,
                ..
            } => (buffer, highlighter),
            _ => return div().id("editor-canvas-non-text"),
        };

        // Process pending edits for tree-sitter
        let pending = buffer.take_pending_edits();
        if !pending.is_empty() {
            highlighter.parse_incremental(buffer.rope(), &pending);
        }

        // Compute visible range
        let first_visible = tab.scroll_offset as usize;
        let last_visible = (first_visible + scroll_lines_per_page + 1).min(buffer.len_lines());

        // Get highlights for visible range
        let highlights = highlighter.highlights_for_range(buffer.rope(), first_visible, last_visible);

        // Collect lines text
        let mut line_texts: Vec<String> = Vec::with_capacity(last_visible - first_visible);
        for line_idx in first_visible..last_visible {
            line_texts.push(buffer.line_text(line_idx));
        }

        let total_lines = buffer.len_lines();
        let tab_size = buffer.tab_size();
        let (cursor_line, cursor_char_col) = buffer.cursor_line_col();
        let cursor_col = buffer.char_col_to_visual_col(cursor_line, cursor_char_col);
        let selection = buffer.selection().cloned();

        // Compute gutter width inline to avoid borrowing self
        let line_count = total_lines;
        let digits = if line_count == 0 {
            1
        } else {
            (line_count as f64).log10().floor() as usize + 1
        };
        let char_width = cache.cell_width.to_f64() as f32;
        let gutter_w = (digits as f32 + 2.0) * char_width;

        // Bracket match
        let bracket_match: Option<(usize, usize)> = buffer.find_matching_bracket();

        // Selection as (start_line, start_visual_col, end_line, end_visual_col) for canvas
        let sel_coords: Option<(usize, usize, usize, usize)> = selection.as_ref().and_then(|s| {
            let range = s.range();
            if range.is_empty() {
                return None;
            }
            let (sl, sc) = buffer.char_to_line_col(range.start);
            let (el, ec) = buffer.char_to_line_col(range.end);
            let sv = buffer.char_col_to_visual_col(sl, sc);
            let ev = buffer.char_col_to_visual_col(el, ec);
            Some((sl, sv, el, ev))
        });

        // Convert search matches from char ranges to (start_line, start_visual_col, end_line, end_visual_col)
        // Only keep matches that overlap the visible range
        // Re-borrow buffer immutably for search match coordinate conversion
        let tab = &self.tabs[tab_idx];
        let buffer = tab.buffer().unwrap();
        let mut search_match_coords: Vec<(usize, usize, usize, usize)> = Vec::new();
        let mut search_current_coord: Option<usize> = None;
        for (mi, m) in self.search_matches.iter().enumerate() {
            let (sl, sc) = buffer.char_to_line_col(m.start);
            let (el, ec) = buffer.char_to_line_col(m.end);
            if el >= first_visible && sl < last_visible {
                if Some(mi) == self.search_current_idx {
                    search_current_coord = Some(search_match_coords.len());
                }
                let sv = buffer.char_col_to_visual_col(sl, sc);
                let ev = buffer.char_col_to_visual_col(el, ec);
                search_match_coords.push((sl, sv, el, ev));
            }
        }

        let handle = cx.entity().downgrade();
        let focus = self.focus_handle.clone();
        let origin_arc = self.canvas_origin.clone();
        let height_arc = self.canvas_height.clone();

        // Mouse handlers
        let h_down = handle.clone();
        let h_right = handle.clone();
        let h_move = handle.clone();
        let h_up = handle.clone();
        let h_scroll = handle.clone();
        let focus_down = focus.clone();

        let container = div()
            .flex_grow()
            .w_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .id("editor-canvas-container")
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, window, cx| {
                    focus_down.focus(window);
                    if let Some(view) = h_down.upgrade() {
                        view.update(cx, |this, cx| {
                            this.handle_mouse_down(event, window, cx);
                        });
                    }
                },
            )
            .on_mouse_down(
                MouseButton::Right,
                move |event: &MouseDownEvent, _window, cx| {
                    if let Some(view) = h_right.upgrade() {
                        view.update(cx, |this, cx| {
                            this.handle_right_click(event, cx);
                        });
                    }
                },
            )
            .on_mouse_move(move |event: &MouseMoveEvent, _window, cx| {
                if let Some(view) = h_move.upgrade() {
                    view.update(cx, |this, cx| {
                        this.handle_mouse_move(event, cx);
                    });
                }
            })
            .on_mouse_up(
                MouseButton::Left,
                move |_event: &MouseUpEvent, _window, cx| {
                    if let Some(view) = h_up.upgrade() {
                        view.update(cx, |this, cx| {
                            this.mouse_selecting = false;
                            this.mouse_click_origin = None;
                            this.scrollbar_dragging = false;
                            cx.notify();
                        });
                    }
                },
            )
            .on_scroll_wheel(move |event: &ScrollWheelEvent, _window, cx| {
                if let Some(view) = h_scroll.upgrade() {
                    view.update(cx, |this, cx| {
                        this.handle_scroll(event, cx);
                    });
                }
            })
            .child(canvas(
                move |bounds, _window, _cx| {
                    // Store canvas origin and height for mouse handlers
                    let ox = bounds.origin.x.to_f64() as f32;
                    let oy = bounds.origin.y.to_f64() as f32;
                    let packed = ((ox.to_bits() as u64) << 32) | (oy.to_bits() as u64);
                    origin_arc.store(packed, std::sync::atomic::Ordering::Relaxed);
                    let h = bounds.size.height.to_f64() as f32;
                    height_arc.store(h.to_bits(), std::sync::atomic::Ordering::Relaxed);
                    (
                        cache,
                        line_texts,
                        highlights,
                        total_lines,
                        first_visible,
                        cursor_line,
                        cursor_col,
                        sel_coords,
                        gutter_w,
                        cursor_blink_on,
                        has_focus,
                        search_match_coords,
                        search_current_coord,
                        tab_size,
                        bracket_match,
                    )
                },
                move |bounds,
                      (
                    cache,
                    line_texts,
                    highlights,
                    total_lines,
                    first_visible,
                    cursor_line,
                    cursor_col,
                    sel_coords,
                    gutter_w,
                    cursor_blink_on,
                    has_focus,
                    search_match_coords,
                    search_current_coord,
                    tab_size,
                    bracket_match,
                ),
                      window,
                      cx| {
                    Self::paint_editor(
                        bounds,
                        &cache,
                        &line_texts,
                        &highlights,
                        total_lines,
                        first_visible,
                        cursor_line,
                        cursor_col,
                        sel_coords,
                        gutter_w,
                        cursor_blink_on,
                        has_focus,
                        &search_match_coords,
                        search_current_coord,
                        tab_size,
                        bracket_match,
                        window,
                        cx,
                    );
                },
            )
            .size_full());

        container
    }

    // -----------------------------------------------------------------------
    // Paint: the actual pixel-level rendering
    // -----------------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    fn paint_editor(
        bounds: Bounds<Pixels>,
        cache: &GlyphCache,
        line_texts: &[String],
        highlights: &[Vec<HighlightSpan>],
        total_lines: usize,
        first_visible: usize,
        cursor_line: usize,
        cursor_col: usize,
        sel_coords: Option<(usize, usize, usize, usize)>,
        gutter_w: f32,
        cursor_blink_on: bool,
        has_focus: bool,
        search_match_coords: &[(usize, usize, usize, usize)],
        search_current_coord: Option<usize>,
        tab_size: usize,
        bracket_match: Option<(usize, usize)>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let cell_w = cache.cell_width;
        let cell_h = cache.cell_height;
        let fs = cache.font_size;
        let gutter_px = px(gutter_w);

        let sel_color = hsla(0.58, 0.6, 0.5, 0.35);
        let search_color = hsla(0.12, 0.8, 0.5, 0.35);
        let search_current_color = hsla(0.12, 0.9, 0.55, 0.55);

        // Paint gutter background
        window.paint_quad(fill(
            Bounds::new(bounds.origin, size(gutter_px, bounds.size.height)),
            ShellDeckColors::line_number_bg(),
        ));

        // Compute digit count for line numbers
        let digits = if total_lines == 0 {
            1
        } else {
            (total_lines as f64).log10().floor() as usize + 1
        };

        // Pass 1: Paint line backgrounds, line numbers, search highlights, selection
        for (ri, line_text) in line_texts.iter().enumerate() {
            let abs_line = first_visible + ri;
            let y = bounds.origin.y + cell_h * ri as f32;

            // Current line highlight
            if abs_line == cursor_line {
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.origin.x + gutter_px, y),
                        size(bounds.size.width - gutter_px, cell_h),
                    ),
                    ShellDeckColors::cursor_line_bg(),
                ));
            }

            // Search match highlights (behind text)
            for (mi, &(sm_sl, sm_sc, sm_el, sm_ec)) in search_match_coords.iter().enumerate() {
                if abs_line < sm_sl || abs_line > sm_el {
                    continue;
                }
                let color = if Some(mi) == search_current_coord {
                    search_current_color
                } else {
                    search_color
                };
                let line_visual_len = visual_line_width(line_text, tab_size);
                let sc = if abs_line == sm_sl { sm_sc } else { 0 };
                let ec = if abs_line == sm_el { sm_ec } else { line_visual_len };
                if sc < ec {
                    let sx = bounds.origin.x + gutter_px + cell_w * sc as f32;
                    let sw = cell_w * (ec - sc) as f32;
                    window.paint_quad(fill(
                        Bounds::new(point(sx, y), size(sw, cell_h)),
                        color,
                    ));
                }
            }

            // Selection overlay (behind text)
            if let Some((sel_start_line, sel_start_col, sel_end_line, sel_end_col)) = sel_coords {
                if abs_line >= sel_start_line && abs_line <= sel_end_line {
                    let line_visual_len = visual_line_width(line_text, tab_size);
                    let start_col = if abs_line == sel_start_line { sel_start_col } else { 0 };
                    let end_col = if abs_line == sel_end_line {
                        sel_end_col
                    } else {
                        line_visual_len + 1
                    };
                    if start_col < end_col {
                        let sel_x = bounds.origin.x + gutter_px + cell_w * start_col as f32;
                        let sel_w = cell_w * (end_col - start_col) as f32;
                        window.paint_quad(fill(
                            Bounds::new(point(sel_x, y), size(sel_w, cell_h)),
                            sel_color,
                        ));
                    }
                }
            }

            // Line number
            let line_num = format!("{:>width$}", abs_line + 1, width = digits);
            let num_color = if abs_line == cursor_line {
                ShellDeckColors::text_primary()
            } else {
                ShellDeckColors::line_number_fg()
            };
            let num_str: SharedString = line_num.into();
            let num_len = num_str.len();
            let shaped_num = window.text_system().shape_line(
                num_str,
                fs,
                &[TextRun {
                    len: num_len,
                    font: cache.base_font.clone(),
                    color: num_color,
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                }],
                None,
            );
            let num_x = bounds.origin.x + px(gutter_w - (digits as f32 + 1.0) * cell_w.to_f64() as f32);
            let _ = shaped_num.paint(point(num_x, y), cell_h, window, cx);
        }

        // Pass 2: Paint text characters on top of backgrounds
        for (ri, line_text) in line_texts.iter().enumerate() {
            let y = bounds.origin.y + cell_h * ri as f32;
            let text_x = bounds.origin.x + gutter_px;
            let line_highlights = highlights.get(ri);

            let mut col = 0usize;
            for (byte_idx, ch) in line_text.char_indices() {
                let x = text_x + cell_w * col as f32;
                let char_byte_end = byte_idx + ch.len_utf8();

                let (fg_color, bold, italic) = if let Some(spans) = line_highlights {
                    Self::color_for_byte_pos(spans, byte_idx, char_byte_end)
                } else {
                    (ShellDeckColors::text_primary(), false, false)
                };

                if ch != ' ' && ch != '\t' {
                    let f = match (bold, italic) {
                        (true, true) => cache.base_font.clone().bold().italic(),
                        (true, false) => cache.base_font.clone().bold(),
                        (false, true) => cache.base_font.clone().italic(),
                        _ => cache.base_font.clone(),
                    };

                    if let Some((font_id, glyph_id)) = cache.lookup(ch, bold, italic) {
                        let _ = window.paint_glyph(
                            point(x, y),
                            font_id,
                            glyph_id,
                            fs,
                            fg_color,
                        );
                    } else {
                        let s: SharedString = ch.to_string().into();
                        let shaped = window.text_system().shape_line(
                            s,
                            fs,
                            &[TextRun {
                                len: ch.len_utf8(),
                                font: f,
                                color: fg_color,
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            }],
                            None,
                        );
                        let _ = shaped.paint(point(x, y), cell_h, window, cx);
                    }
                }

                if ch == '\t' {
                    col += tab_size - (col % tab_size);
                } else {
                    col += 1;
                }
            }
        }

        // Paint cursor
        if has_focus && cursor_blink_on {
            let cursor_visible_line = cursor_line.saturating_sub(first_visible);
            if cursor_line >= first_visible && cursor_line < first_visible + line_texts.len() {
                let cursor_x = bounds.origin.x + gutter_px + cell_w * cursor_col as f32;
                let cursor_y = bounds.origin.y + cell_h * cursor_visible_line as f32;
                window.paint_quad(fill(
                    Bounds::new(
                        point(cursor_x, cursor_y),
                        size(px(2.0), cell_h),
                    ),
                    ShellDeckColors::primary(),
                ));
            }
        }

        // Paint matching bracket highlight
        if let Some((match_line, match_vcol)) = bracket_match {
            if match_line >= first_visible && match_line < first_visible + line_texts.len() {
                let vis_row = match_line - first_visible;
                let bx = bounds.origin.x + gutter_px + cell_w * match_vcol as f32;
                let by = bounds.origin.y + cell_h * vis_row as f32;
                let bracket_bg = hsla(0.58, 0.4, 0.5, 0.25);
                window.paint_quad(fill(
                    Bounds::new(point(bx, by), size(cell_w, cell_h)),
                    bracket_bg,
                ));
            }
        }

        // Paint scrollbar
        if total_lines > 0 {
            let scrollbar_width = px(SCROLLBAR_WIDTH);
            let scrollbar_x = bounds.origin.x + bounds.size.width - scrollbar_width;
            let viewport_lines = line_texts.len().max(1) as f32;
            let thumb_height = (viewport_lines / total_lines as f32) * bounds.size.height;
            let thumb_height = thumb_height.max(px(20.0));
            let thumb_y = bounds.origin.y
                + (first_visible as f32 / total_lines as f32) * bounds.size.height;

            // Track
            window.paint_quad(fill(
                Bounds::new(
                    point(scrollbar_x, bounds.origin.y),
                    size(scrollbar_width, bounds.size.height),
                ),
                hsla(0.0, 0.0, 0.0, 0.1),
            ));

            // Thumb
            window.paint_quad(fill(
                Bounds::new(
                    point(scrollbar_x, thumb_y),
                    size(scrollbar_width, thumb_height),
                ),
                hsla(0.0, 0.0, 0.5, 0.3),
            ));
        }
    }

    fn color_for_byte_pos(
        spans: &[HighlightSpan],
        byte_start: usize,
        byte_end: usize,
    ) -> (Hsla, bool, bool) {
        // Find the most specific (last) span that contains this byte position
        for span in spans.iter().rev() {
            if span.range.start <= byte_start && span.range.end >= byte_end {
                return (span.color, span.bold, span.italic);
            }
        }
        (ShellDeckColors::text_primary(), false, false)
    }

    // -----------------------------------------------------------------------
    // Mouse handlers
    // -----------------------------------------------------------------------

    /// Get the canvas origin (x, y) in window coordinates, set during paint.
    fn canvas_origin_xy(&self) -> (f32, f32) {
        let packed = self.canvas_origin.load(std::sync::atomic::Ordering::Relaxed);
        let ox = f32::from_bits((packed >> 32) as u32);
        let oy = f32::from_bits(packed as u32);
        (ox, oy)
    }

    fn header_height(&self) -> f32 {
        TAB_BAR_HEIGHT
            + if self.search_visible { SEARCH_BAR_HEIGHT } else { 0.0 }
            + if self.replace_visible { REPLACE_BAR_HEIGHT } else { 0.0 }
            + if self.goto_line_visible { GOTO_LINE_BAR_HEIGHT } else { 0.0 }
            + if self.pending_close_tab.is_some() { 32.0 } else { 0.0 }
    }

    fn handle_mouse_down(&mut self, event: &MouseDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        // Dismiss context menu on any click
        if self.context_menu_visible {
            self.context_menu_visible = false;
            cx.notify();
            return;
        }

        // Non-text tabs: no text interaction
        if !self.active_tab().map_or(false, |t| t.is_text()) {
            return;
        }

        self.reset_cursor_blink(cx);

        let cache = match self.glyph_cache.as_ref() {
            Some(c) => c.clone(),
            None => return,
        };

        let tab_idx = self.active_tab_index;
        if tab_idx >= self.tabs.len() {
            return;
        }

        let cell_w = cache.cell_width.to_f64() as f32;
        let cell_h = cache.cell_height.to_f64() as f32;

        let total_lines = self.tabs[tab_idx].buffer().unwrap().len_lines();
        let digits = if total_lines == 0 { 1 } else { (total_lines as f64).log10().floor() as usize + 1 };
        let gutter_w = (digits as f32 + 2.0) * cell_w;

        let pos = event.position;
        let (canvas_ox, canvas_oy) = self.canvas_origin_xy();

        // Position relative to canvas origin
        let rel_x = pos.x.to_f64() as f32 - canvas_ox;
        let rel_y = pos.y.to_f64() as f32 - canvas_oy;

        if rel_y < 0.0 || rel_x < 0.0 {
            return;
        }

        // Check if click is in scrollbar area
        // Canvas fills from canvas_ox to the right edge of the viewport
        let canvas_w = _window.viewport_size().width.to_f64() as f32 - canvas_ox;
        if rel_x >= canvas_w - SCROLLBAR_WIDTH {
            // Scrollbar click
            let canvas_h = f32::from_bits(self.canvas_height.load(std::sync::atomic::Ordering::Relaxed));
            if canvas_h > 0.0 && total_lines > 0 {
                let ratio = rel_y / canvas_h;
                if let Some(tab) = self.tabs.get_mut(tab_idx) {
                    tab.scroll_offset = ratio * total_lines as f32;
                }
                self.clamp_scroll();
                self.scrollbar_dragging = true;
                cx.notify();
            }
            return;
        }

        let text_x = rel_x - gutter_w;
        if text_x < 0.0 {
            return;
        }

        let visual_col = (text_x / cell_w) as usize;
        let row = (rel_y / cell_h) as usize;
        let scroll = self.tabs[tab_idx].scroll_offset;
        let abs_line = row + scroll as usize;
        let char_col = self.tabs[tab_idx].buffer().unwrap().visual_col_to_char_col(abs_line, visual_col);

        let extend = event.modifiers.shift;

        if event.click_count == 2 {
            self.tabs[tab_idx].buffer_mut().unwrap().set_cursor_from_position(abs_line, char_col, false);
            self.tabs[tab_idx].buffer_mut().unwrap().select_word_at_cursor();
        } else {
            self.tabs[tab_idx].buffer_mut().unwrap().set_cursor_from_position(abs_line, char_col, extend);
            self.mouse_selecting = true;
            self.mouse_click_origin = Some((pos.x.to_f64() as f32, pos.y.to_f64() as f32));
        }

        self.ensure_cursor_visible();
        self.reset_cursor_blink(cx);
        cx.notify();
    }

    fn handle_right_click(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        // No context menu for non-text tabs
        if !self.active_tab().map_or(false, |t| t.is_text()) {
            return;
        }
        let x = event.position.x.to_f64() as f32;
        let y = event.position.y.to_f64() as f32;
        self.context_menu_position = (x, y);
        self.context_menu_visible = true;
        cx.notify();
    }

    fn handle_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        // Handle scrollbar dragging
        if self.scrollbar_dragging {
            let tab_idx = self.active_tab_index;
            if tab_idx < self.tabs.len() {
                let (_, canvas_oy) = self.canvas_origin_xy();
                let rel_y = (event.position.y.to_f64() as f32 - canvas_oy).max(0.0);
                let total_lines = self.tabs[tab_idx].buffer().map_or(0, |b| b.len_lines());
                // Use approximate viewport height
                let cell_h = self.glyph_cache.as_ref()
                    .map(|c| c.cell_height.to_f64() as f32)
                    .unwrap_or(20.0);
                let viewport_h = (self.scroll_lines_per_page as f32 * cell_h).max(1.0);
                if total_lines > 0 {
                    let ratio = rel_y / viewport_h;
                    self.tabs[tab_idx].scroll_offset = ratio * total_lines as f32;
                    self.clamp_scroll();
                }
            }
            cx.notify();
            return;
        }

        if !self.mouse_selecting {
            return;
        }

        // Dead zone: ignore small movements near the click origin to prevent
        // accidental cursor displacement during a click
        if let Some((ox, oy)) = self.mouse_click_origin {
            let mx = event.position.x.to_f64() as f32;
            let my = event.position.y.to_f64() as f32;
            let dist_sq = (mx - ox) * (mx - ox) + (my - oy) * (my - oy);
            if dist_sq < 9.0 {
                // Less than 3px movement — ignore
                return;
            }
            // Beyond dead zone — clear origin so we don't check again
            self.mouse_click_origin = None;
        }

        let cache = match self.glyph_cache.as_ref() {
            Some(c) => c.clone(),
            None => return,
        };

        let tab_idx = self.active_tab_index;
        if tab_idx >= self.tabs.len() {
            return;
        }

        let cell_w = cache.cell_width.to_f64() as f32;
        let cell_h = cache.cell_height.to_f64() as f32;

        let total_lines = self.tabs[tab_idx].buffer().map_or(0, |b| b.len_lines());
        let digits = if total_lines == 0 { 1 } else { (total_lines as f64).log10().floor() as usize + 1 };
        let gutter_w = (digits as f32 + 2.0) * cell_w;

        let (canvas_ox, canvas_oy) = self.canvas_origin_xy();
        let rel_x = (event.position.x.to_f64() as f32 - canvas_ox - gutter_w).max(0.0);
        let rel_y = (event.position.y.to_f64() as f32 - canvas_oy).max(0.0);

        let visual_col = (rel_x / cell_w) as usize;
        let row = (rel_y / cell_h) as usize;
        let scroll = self.tabs[tab_idx].scroll_offset as usize;
        let abs_line = row + scroll;
        let char_col = self.tabs[tab_idx].buffer().unwrap().visual_col_to_char_col(abs_line, visual_col);

        self.tabs[tab_idx].buffer_mut().unwrap().set_cursor_from_position(abs_line, char_col, true);

        cx.notify();
    }

    fn handle_scroll(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = match event.delta {
            ScrollDelta::Lines(d) => -d.y * 3.0,
            ScrollDelta::Pixels(d) => {
                let cell_h = self
                    .glyph_cache
                    .as_ref()
                    .map(|c| c.cell_height.to_f64() as f32)
                    .unwrap_or(20.0);
                -d.y.to_f64() as f32 / cell_h
            }
        };

        if let Some(tab) = self.active_tab_mut() {
            tab.scroll_offset += delta;
        }
        self.clamp_scroll();
        cx.notify();
    }

    pub fn is_file_browser_resizing(&self) -> bool {
        self.file_browser_resizing
    }
}

// ---------------------------------------------------------------------------
// Render
// ---------------------------------------------------------------------------
impl Render for FileEditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Update lines per page from window viewport size
        if let Some(ref cache) = self.glyph_cache {
            let cell_h = cache.cell_height.to_f64() as f32;
            if cell_h > 0.0 {
                // Use actual canvas height from prepaint (accurate, includes workspace chrome)
                let stored_h = f32::from_bits(self.canvas_height.load(std::sync::atomic::Ordering::Relaxed));
                if stored_h > 0.0 {
                    self.scroll_lines_per_page = (stored_h / cell_h) as usize;
                } else {
                    // Fallback before first paint: estimate from viewport
                    let viewport_h = window.viewport_size().height.to_f64() as f32;
                    let chrome_h = self.header_height() + STATUS_BAR_HEIGHT;
                    let editor_h = (viewport_h - chrome_h).max(cell_h);
                    self.scroll_lines_per_page = (editor_h / cell_h) as usize;
                }
            }
        }

        // Focus handling
        let focused = self.focus_handle.is_focused(window);
        if focused && self.cursor_blink_task.is_none() {
            self.start_cursor_blink(cx);
        }

        let handle = cx.entity().downgrade();

        let mut container = div()
            .flex()
            .flex_col()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if let Some(view) = handle.upgrade() {
                    view.update(cx, |this, cx| {
                        this.handle_key_down(event, window, cx);
                    });
                }
            });

        // Tab bar
        container = container.child(self.render_tab_bar(cx));

        // Search/replace/goto-line bars (text tabs only)
        let active_is_text = self.active_tab().map_or(true, |t| t.is_text());
        if active_is_text {
            if self.search_visible {
                container = container.child(self.render_search_bar());
            }
            if self.replace_visible {
                container = container.child(self.render_replace_bar(cx));
            }
            if self.goto_line_visible {
                container = container.child(self.render_goto_line_bar());
            }
        }

        // Unsaved changes warning bar
        if let Some(pending_idx) = self.pending_close_tab {
            let tab_name = self.tabs.get(pending_idx)
                .map(|t| t.display_name())
                .unwrap_or_else(|| "untitled".to_string());
            let h_save = cx.entity().downgrade();
            let h_discard = cx.entity().downgrade();
            let h_cancel = cx.entity().downgrade();

            let warning_bar = div()
                .flex()
                .items_center()
                .w_full()
                .h(px(32.0))
                .bg(hsla(0.08, 0.7, 0.5, 0.2))
                .border_b_1()
                .border_color(hsla(0.08, 0.7, 0.5, 0.4))
                .px(px(10.0))
                .gap(px(8.0))
                .text_size(px(12.0))
                .child(
                    div()
                        .text_color(ShellDeckColors::text_primary())
                        .child(format!("\"{}\" has unsaved changes.", tab_name)),
                )
                .child(div().flex_grow())
                .child(
                    div()
                        .id("save-close-btn")
                        .px(px(8.0))
                        .py(px(2.0))
                        .rounded(px(3.0))
                        .bg(ShellDeckColors::primary())
                        .text_color(ShellDeckColors::bg_primary())
                        .text_size(px(11.0))
                        .cursor_pointer()
                        .child("Save & Close")
                        .on_click(move |_event, _window, cx| {
                            if let Some(view) = h_save.upgrade() {
                                view.update(cx, |this, cx| {
                                    if let Some(idx) = this.pending_close_tab {
                                        this.save_and_close_tab(idx, cx);
                                    }
                                });
                            }
                        }),
                )
                .child(
                    div()
                        .id("discard-btn")
                        .px(px(8.0))
                        .py(px(2.0))
                        .rounded(px(3.0))
                        .text_color(hsla(0.0, 0.7, 0.6, 1.0))
                        .text_size(px(11.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                        .child("Discard")
                        .on_click(move |_event, _window, cx| {
                            if let Some(view) = h_discard.upgrade() {
                                view.update(cx, |this, cx| {
                                    if let Some(idx) = this.pending_close_tab {
                                        this.force_close_tab(idx, cx);
                                    }
                                });
                            }
                        }),
                )
                .child(
                    div()
                        .id("cancel-close-btn")
                        .px(px(8.0))
                        .py(px(2.0))
                        .rounded(px(3.0))
                        .text_color(ShellDeckColors::text_muted())
                        .text_size(px(11.0))
                        .cursor_pointer()
                        .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                        .child("Cancel")
                        .on_click(move |_event, _window, cx| {
                            if let Some(view) = h_cancel.upgrade() {
                                view.update(cx, |this, cx| {
                                    this.pending_close_tab = None;
                                    cx.notify();
                                });
                            }
                        }),
                );
            container = container.child(warning_bar);
        }

        // Main editor area: file browser + editor canvas
        let h_resize_move = cx.entity().downgrade();
        let h_resize_up = cx.entity().downgrade();
        let mut editor_area = div()
            .flex()
            .flex_grow()
            .min_h(px(0.0))
            .overflow_hidden()
            .on_mouse_move(move |event: &MouseMoveEvent, _window, cx| {
                if let Some(view) = h_resize_move.upgrade() {
                    view.update(cx, |this, cx| {
                        if this.file_browser_resizing {
                            let x = event.position.x.to_f64() as f32;
                            this.file_browser_width = x.clamp(120.0, 500.0);
                            cx.notify();
                        }
                    });
                }
            })
            .on_mouse_up(MouseButton::Left, move |_event, _window, cx| {
                if let Some(view) = h_resize_up.upgrade() {
                    view.update(cx, |this, cx| {
                        if this.file_browser_resizing {
                            this.file_browser_resizing = false;
                            cx.notify();
                        }
                    });
                }
            });

        // File browser panel
        if self.file_browser_visible {
            let h_browser = cx.entity().downgrade();
            let browser_width = self.file_browser_width;

            let entries = self.file_browser.visible_entries();
            let mut browser_panel = div()
                .flex()
                .flex_col()
                .w(px(browser_width))
                .h_full()
                .bg(ShellDeckColors::bg_sidebar())
                .border_r_1()
                .border_color(ShellDeckColors::border())
                .flex_shrink_0();

            // Browser header
            let browser_header = div()
                .flex()
                .items_center()
                .w_full()
                .h(px(28.0))
                .px(px(8.0))
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .text_size(px(11.0))
                .font_weight(FontWeight::BOLD)
                .text_color(ShellDeckColors::text_muted())
                .child("FILES");

            browser_panel = browser_panel.child(browser_header);

            // File entries (scrollable)
            let mut file_list = div()
                .flex()
                .flex_col()
                .flex_grow()
                .min_h(px(0.0))
                .id("file-browser-list")
                .overflow_y_scroll()
                .py(px(2.0));

            for entry in entries {
                let h = h_browser.clone();
                let path = entry.path.clone();
                let is_dir = entry.is_dir;
                let is_expanded = entry.is_expanded;
                let depth = entry.depth;
                let name = entry.name.clone();

                let mut row = div()
                    .id(SharedString::from(format!("fb-{}", path.display())))
                    .flex()
                    .items_center()
                    .w_full()
                    .h(px(22.0))
                    .px(px(8.0 + depth as f32 * 12.0))
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .hover(|s| s.bg(ShellDeckColors::hover_bg()));

                let icon = if is_dir {
                    if is_expanded {
                        "▾ "
                    } else {
                        "▸ "
                    }
                } else {
                    "  "
                };

                let text_color = if is_dir {
                    ShellDeckColors::text_primary()
                } else {
                    ShellDeckColors::text_muted()
                };

                row = row
                    .text_color(text_color)
                    .child(format!("{}{}", icon, name));

                row = row.on_click(move |_event, _window, cx| {
                    if let Some(view) = h.upgrade() {
                        let p = path.clone();
                        view.update(cx, |this, cx| {
                            if is_dir {
                                this.file_browser.toggle_dir(&p);
                                cx.notify();
                            } else {
                                this.open_file(p, cx);
                            }
                        });
                    }
                });

                file_list = file_list.child(row);
            }

            browser_panel = browser_panel.child(file_list);
            editor_area = editor_area.child(browser_panel);

            // Resize drag handle (mouse_move/up handled on editor_area container)
            let h_resize_down = cx.entity().downgrade();
            let drag_handle = div()
                .id("file-browser-resize-handle")
                .w(px(DRAG_HANDLE_WIDTH))
                .h_full()
                .flex_shrink_0()
                .cursor_col_resize()
                .hover(|s| s.bg(ShellDeckColors::primary()))
                .on_mouse_down(MouseButton::Left, move |_event, _window, cx| {
                    if let Some(view) = h_resize_down.upgrade() {
                        view.update(cx, |this, cx| {
                            this.file_browser_resizing = true;
                            cx.notify();
                        });
                    }
                });
            editor_area = editor_area.child(drag_handle);
        }

        // Editor canvas / non-text content
        let is_text_tab = self.active_tab().map_or(true, |t| t.is_text());
        if is_text_tab {
            let canvas_area = self.render_editor_canvas(window, cx);
            editor_area = editor_area.child(canvas_area);
        } else {
            // Extract data needed for non-text rendering before calling render methods
            let tab = self.active_tab().unwrap();
            match &tab.content {
                TabContent::Image { image_path } => {
                    let path = image_path.clone();
                    editor_area = editor_area.child(Self::render_image_viewer(&path));
                }
                TabContent::Pdf { info } => {
                    let filename = tab.filename.clone();
                    let path = tab.path.clone();
                    let page_count = info.page_count;
                    let file_size = info.file_size;
                    let title = info.title.clone();
                    let author = info.author.clone();
                    let creator = info.creator.clone();
                    let h = cx.entity().downgrade();
                    editor_area = editor_area.child(Self::render_pdf_info(
                        &filename, path.as_deref(), page_count, file_size,
                        title.as_deref(), author.as_deref(), creator.as_deref(), h,
                    ));
                }
                TabContent::Binary { info } => {
                    let filename = tab.filename.clone();
                    let path = tab.path.clone();
                    let file_size = info.file_size;
                    let h = cx.entity().downgrade();
                    editor_area = editor_area.child(Self::render_binary_info(
                        &filename, path.as_deref(), file_size, h,
                    ));
                }
                TabContent::Text { .. } => unreachable!(),
            }
        }

        container = container.child(editor_area);

        // Status bar
        let status = self.render_status_bar();
        container = container.child(status);

        // Context menu overlay
        if self.context_menu_visible {
            container = container.child(self.render_context_menu(cx));
        }

        container
    }
}

impl FileEditorView {
    fn render_context_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (menu_x, menu_y) = self.context_menu_position;
        let handle = cx.entity().downgrade();

        let mut menu = div()
            .absolute()
            .top(px(menu_y))
            .left(px(menu_x))
            .w(px(200.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(4.0))
            .shadow_md()
            .py(px(4.0))
            .text_size(px(12.0));

        for (i, item) in CONTEXT_MENU_ITEMS.iter().enumerate() {
            let h = handle.clone();
            let action = item.action;

            let row = div()
                .id(SharedString::from(format!("ctx-menu-{}", i)))
                .flex()
                .items_center()
                .justify_between()
                .w_full()
                .h(px(26.0))
                .px(px(12.0))
                .cursor_pointer()
                .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                .child(
                    div()
                        .text_color(ShellDeckColors::text_primary())
                        .child(item.label),
                )
                .child(
                    div()
                        .text_color(ShellDeckColors::text_muted())
                        .text_size(px(10.0))
                        .child(item.shortcut),
                )
                .on_click(move |_event, _window, cx| {
                    if let Some(view) = h.upgrade() {
                        view.update(cx, |this, cx| {
                            this.context_menu_visible = false;
                            this.execute_context_action(action, cx);
                        });
                    }
                });

            menu = menu.child(row);
        }

        menu
    }

    fn execute_context_action(&mut self, action: ContextMenuAction, cx: &mut Context<Self>) {
        match action {
            ContextMenuAction::Undo => {
                if let Some(tab) = self.active_tab_mut() {
                    if let TabContent::Text { buffer, highlighter, .. } = &mut tab.content {
                        buffer.undo();
                        highlighter.parse_full(buffer.rope());
                    }
                }
            }
            ContextMenuAction::Redo => {
                if let Some(tab) = self.active_tab_mut() {
                    if let TabContent::Text { buffer, highlighter, .. } = &mut tab.content {
                        buffer.redo();
                        highlighter.parse_full(buffer.rope());
                    }
                }
            }
            ContextMenuAction::Cut => {
                if let Some(tab) = self.active_tab() {
                    if let Some(text) = tab.buffer().and_then(|b| b.selected_text()) {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }
                if let Some(tab) = self.active_tab_mut() {
                    if let TabContent::Text { buffer, highlighter, .. } = &mut tab.content {
                        buffer.delete_selection();
                        highlighter.parse_full(buffer.rope());
                    }
                }
            }
            ContextMenuAction::Copy => {
                if let Some(tab) = self.active_tab() {
                    if let Some(text) = tab.buffer().and_then(|b| b.selected_text()) {
                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                    }
                }
            }
            ContextMenuAction::Paste => {
                if let Some(item) = cx.read_from_clipboard() {
                    if let Some(text) = item.text() {
                        if let Some(tab) = self.active_tab_mut() {
                            if let TabContent::Text { buffer, highlighter, .. } = &mut tab.content {
                                buffer.insert_str(&text);
                                highlighter.parse_full(buffer.rope());
                            }
                        }
                    }
                }
            }
            ContextMenuAction::SelectAll => {
                if let Some(tab) = self.active_tab_mut() {
                    if let Some(buffer) = tab.buffer_mut() {
                        buffer.select_all();
                    }
                }
            }
            ContextMenuAction::ToggleComment => {
                if let Some(prefix) = self
                    .active_tab()
                    .and_then(|t| t.language())
                    .and_then(|l| l.comment_prefix())
                    .map(|s| s.to_string())
                {
                    if let Some(tab) = self.active_tab_mut() {
                        if let TabContent::Text { buffer, highlighter, .. } = &mut tab.content {
                            buffer.toggle_line_comment(&prefix);
                            highlighter.parse_full(buffer.rope());
                        }
                    }
                }
            }
        }
        self.ensure_cursor_visible();
        cx.notify();
    }

    fn render_status_bar(&self) -> impl IntoElement {
        let tab = self.active_tab();
        let is_text = tab.map_or(false, |t| t.is_text());

        let type_name = tab
            .map(|t| t.content_type_name().to_string())
            .unwrap_or_else(|| "Plain Text".to_string());

        let mut bar = div()
            .flex()
            .items_center()
            .w_full()
            .h(px(STATUS_BAR_HEIGHT))
            .bg(ShellDeckColors::bg_sidebar())
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .px(px(10.0))
            .gap(px(16.0))
            .text_size(px(11.0))
            .text_color(ShellDeckColors::text_muted());

        if is_text {
            let (line, col) = tab
                .and_then(|t| t.buffer())
                .map(|b| {
                    let (l, c) = b.cursor_line_col();
                    let vc = b.char_col_to_visual_col(l, c);
                    (l + 1, vc + 1)
                })
                .unwrap_or((1, 1));

            let total_lines = tab
                .and_then(|t| t.buffer())
                .map(|b| b.len_lines())
                .unwrap_or(0);

            let tab_info = tab
                .and_then(|t| t.buffer())
                .map(|b| format!("Spaces: {}", b.tab_size()))
                .unwrap_or_default();

            bar = bar
                .child(format!("Ln {}, Col {}", line, col))
                .child(div().flex_grow())
                .child(tab_info)
                .child(type_name)
                .child(format!("{} lines", total_lines));
        } else {
            bar = bar
                .child(type_name)
                .child(div().flex_grow());
        }

        bar
    }
}

// ---------------------------------------------------------------------------
// Non-text tab rendering + PDF loader
// ---------------------------------------------------------------------------

impl FileEditorView {
    fn render_image_viewer(path: &std::path::Path) -> impl IntoElement {
        div()
            .flex()
            .flex_grow()
            .items_center()
            .justify_center()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .child(
                img(path.to_string_lossy().to_string())
                    .object_fit(ObjectFit::Contain)
                    .max_w_full()
                    .max_h_full(),
            )
    }

    fn render_pdf_info(
        filename: &str,
        path: Option<&std::path::Path>,
        page_count: usize,
        file_size: u64,
        title: Option<&str>,
        author: Option<&str>,
        creator: Option<&str>,
        _handle: WeakEntity<Self>,
    ) -> impl IntoElement {
        let mut card = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.0))
            .p(px(32.0))
            .max_w(px(420.0))
            .rounded(px(8.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border());

        // PDF icon badge
        card = card.child(
            div()
                .px(px(12.0))
                .py(px(6.0))
                .rounded(px(4.0))
                .bg(hsla(0.0, 0.7, 0.55, 0.15))
                .text_color(hsla(0.0, 0.7, 0.55, 1.0))
                .text_size(px(13.0))
                .font_weight(FontWeight::BOLD)
                .child("PDF"),
        );

        // Filename
        card = card.child(
            div()
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ShellDeckColors::text_primary())
                .child(filename.to_string()),
        );

        // Info rows
        card = card.child(Self::info_row("Pages", &page_count.to_string()));
        card = card.child(Self::info_row("Size", &format_file_size(file_size)));
        if let Some(t) = title {
            if !t.is_empty() {
                card = card.child(Self::info_row("Title", t));
            }
        }
        if let Some(a) = author {
            if !a.is_empty() {
                card = card.child(Self::info_row("Author", a));
            }
        }
        if let Some(c) = creator {
            if !c.is_empty() {
                card = card.child(Self::info_row("Creator", c));
            }
        }

        // Open externally button
        if let Some(p) = path {
            let path_owned = p.to_path_buf();
            card = card.child(
                div()
                    .id("open-pdf-external")
                    .mt(px(8.0))
                    .px(px(16.0))
                    .py(px(6.0))
                    .rounded(px(4.0))
                    .bg(ShellDeckColors::primary())
                    .text_color(ShellDeckColors::bg_primary())
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.9))
                    .child("Open in External Viewer")
                    .on_click(move |_event, _window, _cx| {
                        let _ = open::that(&path_owned);
                    }),
            );
        }

        div()
            .flex()
            .flex_grow()
            .items_center()
            .justify_center()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .child(card)
    }

    fn render_binary_info(
        filename: &str,
        path: Option<&std::path::Path>,
        file_size: u64,
        _handle: WeakEntity<Self>,
    ) -> impl IntoElement {
        let ext = std::path::Path::new(filename)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("BIN")
            .to_uppercase();

        let mut card = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(12.0))
            .p(px(32.0))
            .max_w(px(420.0))
            .rounded(px(8.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border());

        // Extension badge
        card = card.child(
            div()
                .px(px(12.0))
                .py(px(6.0))
                .rounded(px(4.0))
                .bg(hsla(0.0, 0.0, 0.5, 0.15))
                .text_color(ShellDeckColors::text_muted())
                .text_size(px(13.0))
                .font_weight(FontWeight::BOLD)
                .child(ext),
        );

        // Filename
        card = card.child(
            div()
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(ShellDeckColors::text_primary())
                .child(filename.to_string()),
        );

        // Message
        card = card.child(
            div()
                .text_size(px(12.0))
                .text_color(ShellDeckColors::text_muted())
                .child("This file cannot be displayed as text."),
        );

        // Info
        card = card.child(Self::info_row("Size", &format_file_size(file_size)));
        if let Some(p) = path {
            card = card.child(Self::info_row("Path", &p.to_string_lossy()));
        }

        // Open externally button
        if let Some(p) = path {
            let path_owned = p.to_path_buf();
            card = card.child(
                div()
                    .id("open-binary-external")
                    .mt(px(8.0))
                    .px(px(16.0))
                    .py(px(6.0))
                    .rounded(px(4.0))
                    .bg(ShellDeckColors::primary())
                    .text_color(ShellDeckColors::bg_primary())
                    .text_size(px(12.0))
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.9))
                    .child("Open with System Application")
                    .on_click(move |_event, _window, _cx| {
                        let _ = open::that(&path_owned);
                    }),
            );
        }

        div()
            .flex()
            .flex_grow()
            .items_center()
            .justify_center()
            .size_full()
            .bg(ShellDeckColors::bg_primary())
            .child(card)
    }

    fn info_row(label: &str, value: &str) -> impl IntoElement {
        div()
            .flex()
            .w_full()
            .gap(px(8.0))
            .text_size(px(12.0))
            .child(
                div()
                    .w(px(60.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!("{}:", label)),
            )
            .child(
                div()
                    .flex_grow()
                    .text_color(ShellDeckColors::text_primary())
                    .child(value.to_string()),
            )
    }

    fn load_pdf_info(path: &std::path::Path) -> Option<PdfInfo> {
        let file_size = std::fs::metadata(path).ok()?.len();
        let doc = lopdf::Document::load(path).ok()?;
        let page_count = doc.get_pages().len();

        let (title, author, creator) = if let Ok(info_dict) = doc
            .trailer
            .get(b"Info")
            .and_then(|v| doc.dereference(v))
            .and_then(|(_, v)| v.as_dict())
        {
            let get_str = |key: &[u8]| -> Option<String> {
                info_dict
                    .get(key)
                    .ok()
                    .and_then(|v| match v {
                        lopdf::Object::String(bytes, _) => {
                            String::from_utf8(bytes.clone()).ok()
                        }
                        _ => None,
                    })
            };
            (get_str(b"Title"), get_str(b"Author"), get_str(b"Creator"))
        } else {
            (None, None, None)
        };

        Some(PdfInfo {
            page_count,
            file_size,
            title,
            author,
            creator,
        })
    }
}

fn format_file_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

/// Compute the visual width of a line accounting for tab expansion.
fn visual_line_width(line: &str, tab_size: usize) -> usize {
    let mut vcol = 0;
    for ch in line.chars() {
        if ch == '\t' {
            vcol += tab_size - (vcol % tab_size);
        } else {
            vcol += 1;
        }
    }
    vcol
}
