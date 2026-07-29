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
use crate::t;
use crate::theme::ShellDeckColors;

mod canvas;
mod chrome;
mod find_in_files;
mod previews;
mod search;
mod tab;

pub use tab::*;

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
const MINIMAP_WIDTH: f32 = 80.0;
const MINIMAP_LINE_HEIGHT: f32 = 2.0;
const MINIMAP_CHAR_WIDTH: f32 = 1.2;

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
    label: String,
    shortcut: String,
    action: ContextMenuAction,
}

/// Modifier prefix for the primary shortcut key — Command (⌘) on macOS,
/// Ctrl+ elsewhere. Terminal has an inline `cfg!` for the same swap; if a
/// third caller shows up, promote to a shared `crate::platform` helper.
fn primary_modifier() -> &'static str {
    if cfg!(target_os = "macos") {
        "\u{2318}"
    } else {
        "Ctrl+"
    }
}

fn context_menu_items() -> Vec<ContextMenuItem> {
    let m = primary_modifier();
    vec![
        ContextMenuItem {
            label: t!("file_editor.context.undo").to_string(),
            shortcut: format!("{m}Z"),
            action: ContextMenuAction::Undo,
        },
        ContextMenuItem {
            label: t!("file_editor.context.redo").to_string(),
            shortcut: format!("{m}Y"),
            action: ContextMenuAction::Redo,
        },
        ContextMenuItem {
            label: t!("file_editor.context.cut").to_string(),
            shortcut: format!("{m}X"),
            action: ContextMenuAction::Cut,
        },
        ContextMenuItem {
            label: t!("file_editor.context.copy").to_string(),
            shortcut: format!("{m}C"),
            action: ContextMenuAction::Copy,
        },
        ContextMenuItem {
            label: t!("file_editor.context.paste").to_string(),
            shortcut: format!("{m}V"),
            action: ContextMenuAction::Paste,
        },
        ContextMenuItem {
            label: t!("file_editor.context.select_all").to_string(),
            shortcut: format!("{m}A"),
            action: ContextMenuAction::SelectAll,
        },
        ContextMenuItem {
            label: t!("file_editor.context.toggle_comment").to_string(),
            shortcut: format!("{m}/"),
            action: ContextMenuAction::ToggleComment,
        },
    ]
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub enum FileEditorEvent {
    TabsChanged,
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
    // Minimap
    pub(crate) minimap_visible: bool,
    pub(crate) minimap_dragging: bool,
    pub(crate) minimap_origin_y: std::sync::Arc<std::sync::atomic::AtomicU32>,
    // Mouse click origin for dead zone (prevents micro-movements from changing cursor)
    pub(crate) mouse_click_origin: Option<(f32, f32)>,
    // Unsaved changes warning
    pub(crate) pending_close_tab: Option<usize>,
    // Cached layout
    pub(crate) font_family: String,
    /// Base text size, driven by the app font-size setting.
    pub(crate) font_size: f32,
    /// Per-editor zoom offset added to the base size (Ctrl +/-/0), in px.
    pub(crate) zoom: f32,
    /// Editor preferences mirroring `AppConfig.editor`. Kept as flat fields so
    /// the paint loop and input handlers read them without cloning a struct.
    /// Updated in bulk via `apply_editor_config`.
    pub(crate) show_line_numbers: bool,
    pub(crate) show_whitespace: bool,
    pub(crate) word_wrap: bool,
    pub(crate) word_wrap_column: usize,
    pub(crate) cursor_blink_enabled: bool,
    pub(crate) insert_spaces: bool,
    pub(crate) editor_tab_size: usize,
    /// Line-height multiplier applied to `font_size` to compute cell height.
    /// VS Code / Zed default is ~1.5 — feels aired-out. 1.4 is our previous
    /// hardcoded value (tighter). Persisted via `EditorConfig.line_height`.
    pub(crate) line_height: f32,
    // Canvas bounds origin (set during prepaint, used by mouse handlers)
    pub(crate) canvas_origin: std::sync::Arc<std::sync::atomic::AtomicU64>,
    // Canvas height in pixels (set during prepaint, used for scroll_lines_per_page)
    pub(crate) canvas_height: std::sync::Arc<std::sync::atomic::AtomicU32>,
    // Find-in-files (search across the workspace tree)
    pub(crate) fif_visible: bool,
    pub(crate) fif_query: String,
    pub(crate) fif_last_query: String,
    pub(crate) fif_results: Vec<FifMatch>,
    pub(crate) fif_selected: usize,
    /// True while a background find-in-files walk is in progress.
    pub(crate) fif_searching: bool,
}

/// One find-in-files match: a file, 1-based line, byte column, and a trimmed
/// preview of the line.
#[derive(Clone)]
pub(crate) struct FifMatch {
    pub path: PathBuf,
    pub line: usize,
    pub preview: String,
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
            minimap_visible: true,
            minimap_dragging: false,
            minimap_origin_y: std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0)),
            mouse_click_origin: None,
            pending_close_tab: None,
            font_family: "JetBrains Mono".to_string(),
            font_size: 14.0,
            zoom: 0.0,
            show_line_numbers: true,
            show_whitespace: false,
            word_wrap: false,
            word_wrap_column: 120,
            cursor_blink_enabled: true,
            insert_spaces: true,
            editor_tab_size: 4,
            line_height: 1.5,
            fif_visible: false,
            fif_query: String::new(),
            fif_last_query: String::new(),
            fif_results: Vec::new(),
            fif_selected: 0,
            fif_searching: false,
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
            && self.tabs[0].buffer().is_none_or(|b| b.len_chars() == 0);
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
            if let (Some(ref path), TabContent::Text { buffer, .. }) = (&tab.path, &mut tab.content)
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
            if let (Some(ref path), TabContent::Text { buffer, .. }) = (&tab.path, &mut tab.content)
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

    /// Effective editor text size = app base size + per-editor zoom, clamped.
    pub(crate) fn effective_font_size(&self) -> f32 {
        (self.font_size + self.zoom).clamp(8.0, 40.0)
    }

    fn ensure_glyph_cache(&mut self, window: &Window) {
        let fs = self.effective_font_size();
        let lh = self.line_height;
        // Rebuild when the cache is missing, was built at a different size,
        // or was built at a different line-height (covers font size, per-tab
        // zoom, and the persisted line-height slider).
        let stale = self
            .glyph_cache
            .as_ref()
            .map(|c| {
                let font_matches = (c.font_size.to_f64() as f32 - fs).abs() < 0.01;
                // cell_height = font_size * clamped_line_height — reverse to
                // check whether the current multiplier matches the cached one.
                let cached_lh =
                    c.cell_height.to_f64() as f32 / (c.font_size.to_f64() as f32).max(1.0);
                let lh_matches = (cached_lh - lh.clamp(1.0, 3.0)).abs() < 0.01;
                !(font_matches && lh_matches)
            })
            .unwrap_or(true);
        if stale {
            self.glyph_cache = Some(Arc::new(GlyphCache::build(
                window.text_system(),
                &self.font_family,
                fs,
                lh,
            )));
        }
    }

    /// Set the editor base text size (driven by the app "App Font Size" setting
    /// so the editor scales with the rest of the UI). The glyph cache rebuilds
    /// automatically on the next render if the effective size changed.
    pub fn set_font_size(&mut self, size: f32) {
        self.font_size = size.clamp(8.0, 40.0);
    }

    /// Zoom the editor text in/out/reset, independent of the app font size.
    pub fn zoom_in(&mut self) {
        self.zoom = (self.zoom + 1.0).min(24.0);
    }
    pub fn zoom_out(&mut self) {
        self.zoom = (self.zoom - 1.0).max(-6.0);
    }
    pub fn zoom_reset(&mut self) {
        self.zoom = 0.0;
    }

    /// Set the editor font family. Rebuilds the glyph cache on the next render.
    pub fn set_font_family(&mut self, family: String) {
        if self.font_family != family && family != "System Default" {
            self.font_family = family;
            self.glyph_cache = None;
        }
    }

    /// Total gutter width, in pixels, for a monospace `cell_w` and a buffer
    /// with `total_lines` lines. Kept in one place so `paint_editor` and the
    /// mouse hit-testers can never disagree — a mismatch here silently breaks
    /// fold-toggle-on-gutter-click.
    ///
    /// Layout (see `.agents/spacing.md`):
    /// `[breakpoint(1)][number(digits+1 pad)][fold(1.5)][code_left_pad(0.5)]`.
    pub(crate) fn compute_gutter_w(&self, cell_w: f32, total_lines: usize) -> f32 {
        let digits = if total_lines == 0 {
            1
        } else {
            (total_lines as f64).log10().floor() as usize + 1
        };
        let breakpoint_col_w = cell_w;
        let fold_col_w = cell_w * 1.5;
        let code_left_pad = cell_w * 0.5;
        if self.show_line_numbers {
            breakpoint_col_w + (digits as f32 + 1.0) * cell_w + fold_col_w + code_left_pad
        } else {
            breakpoint_col_w + fold_col_w + code_left_pad
        }
    }

    /// Sync the whole `EditorConfig` slice into the editor. Called on app
    /// startup (main.rs) and whenever Settings emits `ConfigChanged` — the
    /// workspace forwards the editor slice only (session-state.md rule).
    pub fn apply_editor_config(
        &mut self,
        cfg: &shelldeck_core::config::app_config::EditorConfig,
        cx: &mut Context<Self>,
    ) {
        self.set_font_family(cfg.font_family.clone());
        self.set_font_size(cfg.font_size);
        if (self.line_height - cfg.line_height).abs() > f32::EPSILON {
            self.line_height = cfg.line_height;
            // Invalidate the cache so `ensure_glyph_cache` rebuilds with the
            // new cell height on the next render.
            self.glyph_cache = None;
        }
        self.show_line_numbers = cfg.show_line_numbers;
        self.show_whitespace = cfg.show_whitespace;
        self.word_wrap = cfg.word_wrap;
        self.word_wrap_column = cfg.word_wrap_column;
        self.insert_spaces = cfg.insert_spaces;
        self.editor_tab_size = cfg.tab_size;
        // Propagate tab_size to every open buffer so indent/paint agree.
        for tab in &mut self.tabs {
            if let TabContent::Text { buffer, .. } = &mut tab.content {
                buffer.set_tab_size(cfg.tab_size);
            }
        }
        // Cursor blink: honor the toggle without leaving a stale task alive.
        let was_enabled = self.cursor_blink_enabled;
        self.cursor_blink_enabled = cfg.cursor_blink;
        if !cfg.cursor_blink {
            self.cursor_blink_task = None;
            self.cursor_blink_on = true;
        } else if !was_enabled {
            self.start_cursor_blink(cx);
        }
        cx.notify();
    }

    // -----------------------------------------------------------------------
    // Cursor blink
    // -----------------------------------------------------------------------

    fn start_cursor_blink(&mut self, cx: &mut Context<Self>) {
        // Honor the persisted "cursor blink" toggle: solid cursor when off.
        if !self.cursor_blink_enabled {
            self.cursor_blink_on = true;
            self.cursor_blink_task = None;
            return;
        }
        self.cursor_blink_on = true;
        let handle = cx.entity().downgrade();
        self.cursor_blink_task = Some(cx.spawn(async move |_, cx| loop {
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
        let (cursor_buf_line, _) = buffer.cursor_line_col();
        // Work in visual-row space so folds are accounted for.
        let visible = buffer.visible_lines();
        let cursor_line = match visible.binary_search(&cursor_buf_line) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
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
                // Scroll range is in visible (fold-aware) rows.
                let visible_count = buffer.visible_lines().len();
                let max = (visible_count.saturating_sub(1)) as f32 + half_page;
                tab.scroll_offset = tab.scroll_offset.clamp(0.0, max);
            }
        }
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
                let stored_h = f32::from_bits(
                    self.canvas_height
                        .load(std::sync::atomic::Ordering::Relaxed),
                );
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
        let active_is_text = self.active_tab().is_none_or(|t| t.is_text());
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
            let tab_name = self
                .tabs
                .get(pending_idx)
                .map(|t| t.display_name())
                .unwrap_or_else(|| "untitled".to_string());
            let h_save = cx.entity().downgrade();
            let h_discard = cx.entity().downgrade();
            let h_cancel = cx.entity().downgrade();

            let warning_bar =
                div()
                    .flex()
                    .items_center()
                    .w_full()
                    .h(px(32.0))
                    .bg(ShellDeckColors::warning().opacity(0.2))
                    .border_b_1()
                    .border_color(ShellDeckColors::warning().opacity(0.4))
                    .px(px(10.0))
                    .gap(px(8.0))
                    .text_size(px(12.0))
                    .child(div().text_color(ShellDeckColors::text_primary()).child(
                        t!("file_editor.unsaved_changes", name = tab_name.as_str()).to_string(),
                    ))
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
                            .child(t!("file_editor.unsaved.save_close").to_string())
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
                            .text_color(ShellDeckColors::error())
                            .text_size(px(11.0))
                            .cursor_pointer()
                            .hover(|s| s.bg(ShellDeckColors::hover_bg()))
                            .child(t!("file_editor.unsaved.discard").to_string())
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
                            .child(t!("file_editor.unsaved.cancel").to_string())
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
            // Pane width + resize handle stay in absolute px (the resize drag
            // clamps absolute mouse x); the pane's *content* scales with the UI
            // size via `s` (scaled rems), like the main app sidebar.
            let s = crate::scale::px;
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
                .h(s(28.0))
                .px(s(8.0))
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .text_size(s(11.0))
                .font_weight(FontWeight::BOLD)
                .text_color(ShellDeckColors::text_muted())
                .child(t!("file_editor.browser.files").to_string());

            browser_panel = browser_panel.child(browser_header);

            // File entries (scrollable)
            let mut file_list = div()
                .flex()
                .flex_col()
                .flex_grow()
                .min_h(px(0.0))
                .id("file-browser-list")
                .overflow_y_scroll()
                .py(s(2.0));

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
                    .h(s(22.0))
                    .px(s(8.0 + depth as f32 * 12.0))
                    .text_size(s(12.0))
                    .cursor_pointer()
                    .hover(|st| st.bg(ShellDeckColors::hover_bg()));

                let icon = if is_dir {
                    if is_expanded {
                        "⌄ "
                    } else {
                        "› "
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

            // Resize drag handle (mouse_move/up handled on editor_area
            // container). Default bg matches the sidebar so the 4px slot
            // reads as one continuous surface with the file browser on the
            // left and the gutter on the right — otherwise the transparent
            // handle leaks the code panel's `bg_primary` through and prints
            // a foreign-colored strip between the two sidebar-toned zones
            // (see `.agents/spacing.md`).
            let h_resize_down = cx.entity().downgrade();
            let drag_handle = div()
                .id("file-browser-resize-handle")
                .w(px(DRAG_HANDLE_WIDTH))
                .h_full()
                .flex_shrink_0()
                .bg(ShellDeckColors::bg_sidebar())
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
        let is_text_tab = self.active_tab().is_none_or(|t| t.is_text());
        if is_text_tab {
            let canvas_area = self.render_editor_canvas(window, cx);
            editor_area = editor_area.child(canvas_area);
            // Minimap
            if self.minimap_visible {
                let minimap = self.render_minimap(window, cx);
                editor_area = editor_area.child(minimap);
            }
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
                        &filename,
                        path.as_deref(),
                        page_count,
                        file_size,
                        title.as_deref(),
                        author.as_deref(),
                        creator.as_deref(),
                        h,
                    ));
                }
                TabContent::Binary { info } => {
                    let filename = tab.filename.clone();
                    let path = tab.path.clone();
                    let file_size = info.file_size;
                    let h = cx.entity().downgrade();
                    editor_area = editor_area.child(Self::render_binary_info(
                        &filename,
                        path.as_deref(),
                        file_size,
                        h,
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

        // Find-in-files overlay
        if self.fif_visible {
            container = container.child(self.render_find_in_files(cx));
        }

        container
    }
}

/// Count leading whitespace visual columns.
fn count_leading_vcols(line: &str, tab_size: usize) -> usize {
    let mut vcol = 0;
    for ch in line.chars() {
        if ch == ' ' {
            vcol += 1;
        } else if ch == '\t' {
            vcol += tab_size - (vcol % tab_size);
        } else {
            break;
        }
    }
    vcol
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
