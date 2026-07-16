use std::collections::HashMap;

use crate::scale::px;
use adabraka_ui::overlays::popover_menu::{PopoverMenu, PopoverMenuItem};
use adabraka_ui::prelude::*;
use gpui::*;
use shelldeck_core::models::execution::ExecutionRecord;
use shelldeck_core::models::script::{Script, ScriptCategory, ScriptLanguage, ScriptTarget};
use uuid::Uuid;

use crate::editor_buffer::EditorBuffer;
use crate::icons::{lucide_icon, lucide_path, script_category_chip, script_language_icon};
use crate::syntax::highlight::render_code_block_with_language;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum ScriptEvent {
    RunScript(Script),
    StopScript,
    AddScript,
    EditScript(Uuid),
    UpdateScript(Script),
    ClearOutput,
    ToggleFavorite(Uuid),
    TogglePinToToolbar(Uuid),
    DeleteScript(Uuid),
    ImportTemplate(String),
    RunScriptById(Uuid),
    GenerateWithAi(Uuid),
    ExplainWithAi(Uuid),
    ReviewWithAi(Uuid),
    FixWithAi(Uuid),
}

impl EventEmitter<ScriptEvent> for ScriptEditorView {}

pub struct ScriptEditorView {
    pub scripts: Vec<Script>,
    pub selected_script: Option<Uuid>,
    pub execution_output: Vec<String>,
    pub running_script_id: Option<Uuid>,
    pub history: Vec<ExecutionRecord>,
    // Run target picker — adabraka PopoverMenu anchored to the split Run button
    run_target_menu_open: bool,
    run_target_btn_bounds: Option<Bounds<Pixels>>,
    run_target_connections: Vec<(Uuid, String)>,
    // Output panel state
    output_panel_height: f32,
    output_resizing: bool,
    // Inline editing state
    inline_editing: bool,
    inline_buffer: EditorBuffer,
    inline_script_id: Option<Uuid>,
    ai_generation_enabled: bool,
    focus_handle: FocusHandle,
    // New: filtering state
    selected_category: Option<ScriptCategory>,
    search_query: String,
    show_favorites_only: bool,
    // Template browser toggle (handled in workspace)
    pub template_browser_open: bool,
    // Last-used variable values per script (script_id -> name -> value)
    pub last_var_values: HashMap<Uuid, HashMap<String, String>>,
    /// Open kebab (⋮) menu: script + click position (window coords).
    kebab_menu: Option<(Uuid, Point<Pixels>)>,
    /// Compact AI actions menu in the selected script toolbar.
    ai_actions_menu: Option<(Uuid, Point<Pixels>)>,
}

impl ScriptEditorView {
    pub fn is_running(&self) -> bool {
        self.running_script_id.is_some()
    }

    pub fn is_output_resizing(&self) -> bool {
        self.output_resizing
    }

    pub fn stop_output_resizing(&mut self) {
        self.output_resizing = false;
    }

    pub fn set_output_height(&mut self, h: f32) {
        self.output_panel_height = h.clamp(80.0, 600.0);
    }

    pub fn set_connections(&mut self, conns: Vec<(Uuid, String)>) {
        self.run_target_connections = conns;
    }

    fn close_run_target_menu(&mut self, cx: &mut Context<Self>) {
        self.run_target_menu_open = false;
        cx.notify();
    }

    fn toggle_run_target_menu(&mut self, cx: &mut Context<Self>) {
        self.run_target_menu_open = !self.run_target_menu_open;
        cx.notify();
    }

    fn run_target_menu_position(&self) -> Option<Point<Pixels>> {
        let bounds = self.run_target_btn_bounds?;
        const MENU_W: f32 = 240.0;
        Some(point(
            bounds.origin.x + bounds.size.width - gpui::px(MENU_W),
            bounds.origin.y + bounds.size.height + gpui::px(4.0),
        ))
    }

    pub fn start_inline_edit(&mut self, script_id: Uuid) {
        if let Some(script) = self.scripts.iter().find(|s| s.id == script_id) {
            self.inline_buffer = EditorBuffer::from_text(script.body.clone());
            self.inline_editing = true;
            self.inline_script_id = Some(script_id);
        }
    }

    pub fn cancel_inline_edit(&mut self) {
        self.inline_editing = false;
        self.inline_buffer = EditorBuffer::new();
        self.inline_script_id = None;
    }

    pub fn save_inline_edit(&mut self, cx: &mut Context<Self>) {
        if let Some(script_id) = self.inline_script_id {
            if let Some(script) = self.scripts.iter().find(|s| s.id == script_id).cloned() {
                let mut updated = script;
                updated.body = self.inline_buffer.text().to_string();
                cx.emit(ScriptEvent::UpdateScript(updated));
            }
        }
        self.cancel_inline_edit();
    }

    fn handle_inline_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;

        // Ctrl+S: save
        if key == "s" && mods.secondary() {
            self.save_inline_edit(cx);
            cx.notify();
            return;
        }

        match key {
            "escape" => {
                self.cancel_inline_edit();
                cx.notify();
                return;
            }
            "left" => {
                self.inline_buffer.move_left();
                cx.notify();
                return;
            }
            "right" => {
                self.inline_buffer.move_right();
                cx.notify();
                return;
            }
            "up" => {
                self.inline_buffer.move_up();
                cx.notify();
                return;
            }
            "down" => {
                self.inline_buffer.move_down();
                cx.notify();
                return;
            }
            "home" => {
                self.inline_buffer.move_home();
                cx.notify();
                return;
            }
            "end" => {
                self.inline_buffer.move_end();
                cx.notify();
                return;
            }
            "enter" => {
                self.inline_buffer.insert_newline();
                cx.notify();
                return;
            }
            "backspace" => {
                self.inline_buffer.backspace();
                cx.notify();
                return;
            }
            "delete" => {
                self.inline_buffer.delete();
                cx.notify();
                return;
            }
            "tab" => {
                self.inline_buffer.insert_tab();
                cx.notify();
                return;
            }
            _ => {}
        }

        // Ctrl+V paste
        if key == "v" && mods.secondary() {
            if let Some(item) = cx.read_from_clipboard() {
                if let Some(text) = item.text() {
                    self.inline_buffer.insert_str(&text);
                    cx.notify();
                }
            }
            return;
        }

        // Printable characters
        if let Some(ref kc) = event.keystroke.key_char {
            if !mods.control && !mods.alt {
                for c in kc.chars() {
                    self.inline_buffer.insert_char(c);
                }
                cx.notify();
                return;
            }
        }

        // Single-char key fallback
        if key.len() == 1 && !mods.control && !mods.alt {
            for c in key.chars() {
                self.inline_buffer.insert_char(c);
            }
            cx.notify();
        }
    }

    /// Handle search input key events when search is active but not inline editing.
    fn handle_search_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;

        match key {
            "escape" => {
                self.search_query.clear();
                cx.notify();
                return;
            }
            "backspace" => {
                self.search_query.pop();
                cx.notify();
                return;
            }
            _ => {}
        }

        // Ctrl+V paste into search
        if key == "v" && mods.secondary() {
            if let Some(item) = cx.read_from_clipboard() {
                if let Some(text) = item.text() {
                    self.search_query.push_str(&text);
                    cx.notify();
                }
            }
            return;
        }

        // Printable characters
        if let Some(ref kc) = event.keystroke.key_char {
            if !mods.control && !mods.alt {
                self.search_query.push_str(kc);
                cx.notify();
                return;
            }
        }

        if key.len() == 1 && !mods.control && !mods.alt {
            self.search_query.push_str(key);
            cx.notify();
        }
    }

    /// Filter scripts based on current category, search query, and favorites.
    fn filtered_scripts(&self) -> Vec<&Script> {
        let query_lower = self.search_query.to_lowercase();
        self.scripts
            .iter()
            .filter(|s| {
                // Category filter
                if let Some(cat) = self.selected_category {
                    if s.category != cat {
                        return false;
                    }
                }
                // Favorites filter
                if self.show_favorites_only && !s.is_favorite {
                    return false;
                }
                // Search filter
                if !query_lower.is_empty() {
                    let name_match = s.name.to_lowercase().contains(&query_lower);
                    let desc_match = s
                        .description
                        .as_ref()
                        .map(|d| d.to_lowercase().contains(&query_lower))
                        .unwrap_or(false);
                    if !name_match && !desc_match {
                        return false;
                    }
                }
                true
            })
            .collect()
    }
}

impl ScriptEditorView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        // Start with built-in scripts
        let scripts = vec![
            Script::builtin_disk_usage(),
            Script::builtin_tail_logs(),
            Script::builtin_system_info(),
        ];

        Self {
            scripts,
            selected_script: None,
            execution_output: Vec::new(),
            running_script_id: None,
            history: Vec::new(),
            run_target_menu_open: false,
            run_target_btn_bounds: None,
            run_target_connections: Vec::new(),
            output_panel_height: 200.0,
            output_resizing: false,
            inline_editing: false,
            inline_buffer: EditorBuffer::new(),
            inline_script_id: None,
            ai_generation_enabled: false,
            focus_handle: cx.focus_handle(),
            selected_category: None,
            search_query: String::new(),
            show_favorites_only: false,
            template_browser_open: false,
            last_var_values: HashMap::new(),
            kebab_menu: None,
            ai_actions_menu: None,
        }
    }

    pub fn add_script(&mut self, script: Script) {
        self.scripts.push(script);
    }

    pub fn append_output(&mut self, text: &str) {
        for line in text.lines() {
            self.execution_output.push(line.to_string());
        }
    }

    fn selected(&self) -> Option<&Script> {
        self.selected_script
            .and_then(|id| self.scripts.iter().find(|s| s.id == id))
    }

    pub fn ai_context_data(&self) -> serde_json::Value {
        let selected = self.selected();
        serde_json::json!({
            "script": selected.map(|script| serde_json::json!({
                "name": script.name,
                "description": script.description,
                "language": script.language.label(),
                "body": script.body,
                "working_dir": script.working_dir,
                "tags": script.tags,
            })),
            "recent_output": self.execution_output.iter().rev().take(80).rev().cloned().collect::<Vec<_>>(),
        })
    }

    fn selected_failed_execution(&self) -> Option<&ExecutionRecord> {
        let script_id = self.selected_script?;
        self.history
            .iter()
            .rev()
            .find(|record| record.script_id == script_id && record.failed())
            .filter(|failed| {
                !self.history.iter().rev().any(|record| {
                    record.script_id == script_id && record.started_at > failed.started_at
                })
            })
    }

    pub fn ai_fix_context_data(&self) -> serde_json::Value {
        let failed = self.selected_failed_execution();
        serde_json::json!({
            "script": self.ai_context_data(),
            "failed_execution": failed.map(|record| serde_json::json!({
                "exit_code": record.exit_code,
                "output": record.output_log,
                "connection_id": record.connection_id,
                "started_at": record.started_at,
            })),
        })
    }

    pub fn set_ai_generation_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.ai_generation_enabled = enabled;
        cx.notify();
    }

    pub fn apply_generated_body(&mut self, script_id: Uuid, body: String, cx: &mut Context<Self>) {
        if self.scripts.iter().any(|script| script.id == script_id) {
            self.selected_script = Some(script_id);
            self.inline_script_id = Some(script_id);
            self.inline_buffer = EditorBuffer::from_text(body);
            self.inline_editing = true;
            cx.notify();
        }
    }

    fn render_language_badge(lang: &ScriptLanguage) -> Div {
        let (r, g, b) = lang.badge_color();
        let color = gpui::hsla(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0);
        div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(22.0))
            .h(px(16.0))
            .rounded(px(3.0))
            .bg(ShellDeckColors::selected_bg())
            .border_1()
            .border_color(color.opacity(0.45))
            .child(script_language_icon(lang.clone(), 11.0))
    }

    fn render_script_list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut list = div()
            .flex()
            .flex_col()
            .w(px(260.0))
            .h_full()
            .bg(ShellDeckColors::bg_sidebar())
            .border_r_1()
            .border_color(ShellDeckColors::border());

        // Header with title and buttons
        list = list.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .px(px(12.0))
                .py(px(10.0))
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .child(t!("scripts.title").to_string()),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        // Favorites toggle
                        .child(
                            div()
                                .id("fav-toggle-btn")
                                .cursor_pointer()
                                .text_size(px(14.0))
                                .text_color(if self.show_favorites_only {
                                    ShellDeckColors::warning()
                                } else {
                                    ShellDeckColors::text_muted()
                                })
                                .hover(|el| el.text_color(ShellDeckColors::warning()))
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.show_favorites_only = !this.show_favorites_only;
                                    cx.notify();
                                }))
                                .child("*"),
                        )
                        // Browse templates button
                        .child(
                            div()
                                .id("browse-templates-btn")
                                .cursor_pointer()
                                .text_size(px(11.0))
                                .px(px(6.0))
                                .py(px(2.0))
                                .rounded(px(4.0))
                                .text_color(ShellDeckColors::text_muted())
                                .hover(|el| {
                                    el.bg(ShellDeckColors::hover_bg())
                                        .text_color(ShellDeckColors::text_primary())
                                })
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.template_browser_open = true;
                                    cx.notify();
                                }))
                                .child(t!("scripts.templates").to_string()),
                        )
                        // Add script button
                        .child(
                            div()
                                .id("add-script-btn")
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .text_color(ShellDeckColors::text_muted())
                                .hover(|el| el.text_color(ShellDeckColors::primary()))
                                .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                    cx.emit(ScriptEvent::AddScript);
                                }))
                                .child(
                                    svg()
                                        .path("icons/lucide/plus.svg")
                                        .size(px(14.0))
                                        .text_color(ShellDeckColors::text_muted()),
                                ),
                        ),
                ),
        );

        // Search bar
        list = list.child(
            div()
                .px(px(8.0))
                .py(px(6.0))
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .child(
                    div()
                        .id("script-search-box")
                        .w_full()
                        .px(px(8.0))
                        .py(px(4.0))
                        .rounded(px(4.0))
                        .bg(ShellDeckColors::bg_primary())
                        .border_1()
                        .border_color(ShellDeckColors::border())
                        .text_size(px(12.0))
                        .cursor_text()
                        .on_click(cx.listener(|_this, _: &ClickEvent, _, _cx| {
                            // Focus is handled by key_down interception
                        }))
                        .child(if self.search_query.is_empty() {
                            div()
                                .text_color(ShellDeckColors::text_muted())
                                .child(t!("scripts.search_placeholder").to_string())
                        } else {
                            div()
                                .text_color(ShellDeckColors::text_primary())
                                .child(self.search_query.clone())
                        }),
                ),
        );

        // Category filter tabs
        {
            let mut tabs = div()
                .flex()
                .flex_wrap()
                .gap(px(2.0))
                .px(px(8.0))
                .py(px(6.0))
                .border_b_1()
                .border_color(ShellDeckColors::border());

            // "All" tab
            let all_selected = self.selected_category.is_none();
            let mut all_tab = div()
                .id("cat-all")
                .px(px(6.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .text_size(px(10.0))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.selected_category = None;
                    cx.notify();
                }));

            if all_selected {
                all_tab = all_tab
                    .bg(ShellDeckColors::primary().opacity(0.15))
                    .text_color(ShellDeckColors::primary());
            } else {
                all_tab = all_tab
                    .text_color(ShellDeckColors::text_muted())
                    .hover(|el| el.bg(ShellDeckColors::hover_bg()));
            }

            tabs = tabs.child(
                all_tab.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(3.0))
                        .child(lucide_icon(
                            "filter",
                            10.0,
                            if all_selected {
                                ShellDeckColors::primary()
                            } else {
                                ShellDeckColors::text_muted()
                            },
                        ))
                        .child(t!("scripts.category_all").to_string()),
                ),
            );

            // Category tabs (only show categories that have scripts)
            let used_categories: std::collections::HashSet<ScriptCategory> =
                self.scripts.iter().map(|s| s.category).collect();

            for cat in ScriptCategory::ALL {
                if !used_categories.contains(cat) && self.selected_category != Some(*cat) {
                    continue;
                }
                let selected = self.selected_category == Some(*cat);
                let cat_val = *cat;
                let mut tab = div()
                    .id(ElementId::from(SharedString::from(format!(
                        "cat-{}",
                        cat.label()
                    ))))
                    .px(px(6.0))
                    .py(px(2.0))
                    .rounded(px(4.0))
                    .text_size(px(10.0))
                    .font_weight(FontWeight::MEDIUM)
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.selected_category = Some(cat_val);
                        cx.notify();
                    }));

                if selected {
                    tab = tab
                        .bg(ShellDeckColors::primary().opacity(0.15))
                        .text_color(ShellDeckColors::primary());
                } else {
                    tab = tab
                        .text_color(ShellDeckColors::text_muted())
                        .hover(|el| el.bg(ShellDeckColors::hover_bg()));
                }

                let icon_color = if selected {
                    ShellDeckColors::primary()
                } else {
                    ShellDeckColors::text_muted()
                };
                tabs = tabs.child(tab.child(script_category_chip(cat_val, icon_color)));
            }

            list = list.child(tabs);
        }

        // Script items
        let filtered = self.filtered_scripts();

        if filtered.is_empty() {
            list = list.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .flex_grow()
                    .gap(px(8.0))
                    .py(px(24.0))
                    .px(px(12.0))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(if self.scripts.is_empty() {
                                t!("scripts.empty.none").to_string()
                            } else {
                                t!("scripts.empty.no_match").to_string()
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("scripts.empty_hint").to_string()),
                    ),
            );
        }

        for script in filtered {
            let script_id = script.id;
            let is_selected = self.selected_script == Some(script.id);
            let target_label = match &script.target {
                ScriptTarget::Local => t!("scripts.target.local").to_string(),
                ScriptTarget::Remote(_) => t!("scripts.target.remote").to_string(),
                ScriptTarget::AskOnRun => t!("scripts.target.ask").to_string(),
            };
            let is_favorite = script.is_favorite;
            let is_pinned = script.pinned_to_toolbar;
            let lang = script.language.clone();

            let mut item_el = div()
                .id(ElementId::from(SharedString::from(format!(
                    "script-{}",
                    script_id
                ))))
                .group("script-item")
                .flex()
                .flex_col()
                .gap(px(2.0))
                .w_full()
                .px(px(12.0))
                .py(px(8.0))
                .cursor_pointer()
                .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                    if event.click_count() >= 2 {
                        this.start_inline_edit(script_id);
                        this.focus_handle.focus(window);
                    } else {
                        this.selected_script = Some(script_id);
                    }
                    cx.notify();
                }));

            if is_selected {
                item_el = item_el
                    .bg(ShellDeckColors::primary().opacity(0.12))
                    .border_l_2()
                    .border_color(ShellDeckColors::primary());
            } else {
                item_el = item_el.hover(|el| el.bg(ShellDeckColors::hover_bg()));
            }

            // Row 1: language badge + name + favorite star
            let row1 = div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(Self::render_language_badge(&lang))
                .child(
                    div()
                        .flex_grow()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(13.0))
                        .text_color(ShellDeckColors::text_primary())
                        .font_weight(FontWeight::MEDIUM)
                        .child(script.name.clone()),
                )
                // Row actions — tight cluster so star / pin / kebab don't drift apart.
                .child({
                    let star = div()
                        .id(ElementId::from(SharedString::from(format!(
                            "fav-{}",
                            script_id
                        ))))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(16.0))
                        .h(px(16.0))
                        .cursor_pointer()
                        .text_size(px(13.0))
                        .text_color(if is_favorite {
                            ShellDeckColors::warning()
                        } else {
                            ShellDeckColors::text_muted().opacity(0.6)
                        })
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(ScriptEvent::ToggleFavorite(script_id));
                        }))
                        .child(if is_favorite { "\u{2605}" } else { "\u{2606}" });
                    let pin = div()
                        .id(ElementId::from(SharedString::from(format!(
                            "pin-{}",
                            script_id
                        ))))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(16.0))
                        .h(px(16.0))
                        .cursor_pointer()
                        .text_color(if is_pinned {
                            ShellDeckColors::primary()
                        } else {
                            ShellDeckColors::text_muted().opacity(0.6)
                        })
                        .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(ScriptEvent::TogglePinToToolbar(script_id));
                        }))
                        .child(
                            svg()
                                .path(if is_pinned {
                                    "icons/lucide/pin.svg"
                                } else {
                                    "images/pin-outline.svg"
                                })
                                .size(px(11.0))
                                .text_color(if is_pinned {
                                    ShellDeckColors::primary()
                                } else {
                                    ShellDeckColors::text_muted().opacity(0.6)
                                }),
                        );
                    let kebab = div()
                        .id(ElementId::from(SharedString::from(format!(
                            "script-kebab-{}",
                            script_id
                        ))))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .w(px(16.0))
                        .h(px(16.0))
                        .rounded(px(4.0))
                        .opacity(0.35)
                        .group_hover("script-item", |el| el.opacity(1.0))
                        .cursor_pointer()
                        .hover(|el| {
                            el.bg(ShellDeckColors::hover_bg())
                                .text_color(ShellDeckColors::text_primary())
                        })
                        .on_click(cx.listener(move |this, event: &ClickEvent, _window, cx| {
                            cx.stop_propagation();
                            this.kebab_menu = Some((script_id, event.position()));
                            cx.notify();
                        }))
                        .child(
                            svg()
                                .path("icons/lucide/ellipsis-vertical.svg")
                                .size(px(14.0))
                                .text_color(ShellDeckColors::text_muted()),
                        );
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap(px(2.0))
                        .child(star)
                        .child(pin)
                        .child(kebab)
                });

            // Row 2: target badge + run button (on hover)
            let row2 = div()
                .flex()
                .items_center()
                .gap(px(6.0))
                .child(
                    div()
                        .text_size(px(10.0))
                        .px(px(4.0))
                        .py(px(1.0))
                        .rounded(px(3.0))
                        .bg(ShellDeckColors::badge_bg())
                        .text_color(ShellDeckColors::text_muted())
                        .child(target_label.to_string()),
                )
                // Run count if any
                .child(if script.run_count > 0 {
                    div()
                        .text_size(px(9.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(format!("{}x", script.run_count))
                } else {
                    div()
                })
                .child(div().flex_grow());

            list = list.child(item_el.child(row1).child(row2));
        }

        list
    }

    fn render_editor(&self, script: &Script, cx: &mut Context<Self>) -> impl IntoElement {
        let script_clone = script.clone();
        let script_id = script.id;
        let is_inline_editing = self.inline_editing && self.inline_script_id == Some(script.id);
        let lang = &script.language;

        let mut script_info = div().flex().flex_col().min_w_0().overflow_hidden().child(
            div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .child(Self::render_language_badge(lang))
                .child(
                    div()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ShellDeckColors::text_primary())
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(script.name.clone()),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .px(px(4.0))
                        .py(px(1.0))
                        .rounded(px(3.0))
                        .bg(ShellDeckColors::badge_bg())
                        .text_color(ShellDeckColors::text_muted())
                        .child(script.category.label().to_string()),
                ),
        );

        if let Some(ref desc) = script.description {
            script_info = script_info.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(desc.clone()),
            );
        }

        // Build run buttons area
        let run_buttons = if self.is_running() {
            div().id("stop-script-area").flex().gap(px(4.0)).child(
                div()
                    .id("stop-script-btn")
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                        cx.emit(ScriptEvent::StopScript);
                    }))
                    .child(
                        Button::new("stop-script", t!("scripts.stop").to_string())
                            .variant(ButtonVariant::Destructive),
                    ),
            )
        } else {
            let menu_open = self.run_target_menu_open;
            let entity = cx.entity();
            div()
                .id("run-split-btn")
                .relative()
                .flex()
                .flex_shrink_0()
                .items_center()
                .rounded(px(6.0))
                .overflow_hidden()
                .bg(ShellDeckColors::primary())
                .border_1()
                .border_color(if menu_open {
                    gpui::white().opacity(0.45)
                } else {
                    ShellDeckColors::primary()
                })
                .child({
                    let entity = entity.clone();
                    canvas(
                        move |bounds, _, cx| {
                            entity.update(cx, |this, _| {
                                this.run_target_btn_bounds = Some(bounds);
                            });
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full()
                })
                .child(
                    div()
                        .id("run-script-btn")
                        .flex()
                        .items_center()
                        .px(px(14.0))
                        .py(px(6.0))
                        .text_size(px(13.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(gpui::white())
                        .cursor_pointer()
                        .hover(|el| el.bg(gpui::white().opacity(0.08)))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if !this.is_running() {
                                if matches!(script_clone.target, ScriptTarget::AskOnRun) {
                                    this.toggle_run_target_menu(cx);
                                } else {
                                    cx.emit(ScriptEvent::RunScript(script_clone.clone()));
                                }
                            }
                        }))
                        .child(t!("scripts.run").to_string()),
                )
                .child(div().w(px(1.0)).bg(gpui::white().opacity(0.28)))
                .child(
                    div()
                        .id("run-target-dropdown-btn")
                        .flex()
                        .items_center()
                        .justify_center()
                        .px(px(8.0))
                        .py(px(6.0))
                        .cursor_pointer()
                        .hover(|el| el.bg(gpui::white().opacity(0.08)))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.toggle_run_target_menu(cx);
                        }))
                        .child(
                            svg()
                                .path(lucide_path("chevron-down"))
                                .size(px(14.0))
                                .text_color(gpui::white()),
                        ),
                )
        };

        // Edit/Cancel button in header
        let edit_button = if is_inline_editing {
            div()
                .id("edit-script-btn")
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.cancel_inline_edit();
                    cx.notify();
                }))
                .child(
                    Button::new("cancel-edit-script", t!("scripts.cancel").to_string())
                        .variant(ButtonVariant::Ghost),
                )
        } else {
            div()
                .id("edit-script-btn")
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.start_inline_edit(script_id);
                    this.focus_handle.focus(window);
                    cx.notify();
                }))
                .child(
                    Button::new("edit-script", t!("scripts.edit").to_string())
                        .variant(ButtonVariant::Ghost),
                )
        };
        let ai_actions = self.ai_generation_enabled.then(|| {
            div()
                .flex()
                .items_center()
                .flex_shrink_0()
                .gap(px(6.0))
                .child(
                    Button::new(
                        "generate-script-ai",
                        t!("ai.workflow.generate_script").to_string(),
                    )
                    .variant(ButtonVariant::Ai)
                    .size(ButtonSize::Sm)
                    .icon(IconSource::from("sparkles"))
                    .disabled(is_inline_editing)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(ScriptEvent::GenerateWithAi(script_id));
                    })),
                )
                .child(
                    Button::new("script-ai-more", "")
                        .variant(ButtonVariant::Ai)
                        .size(ButtonSize::Sm)
                        .tooltip(t!("ai.workflow.more_actions").to_string())
                        .icon(IconSource::from("ellipsis"))
                        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                            this.ai_actions_menu = Some((script_id, event.position()));
                            cx.notify();
                        })),
                )
        });

        // Header bar
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(16.0))
            .py(px(10.0))
            .border_b_1()
            .border_color(ShellDeckColors::border())
            .child(script_info)
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .gap(px(8.0))
                    .children(ai_actions)
                    .child(edit_button)
                    .child(run_buttons),
            );

        // Dependency status bar (if script has dependencies)
        let dep_bar = if !script.dependencies.is_empty() {
            let mut bar = div()
                .flex()
                .items_center()
                .gap(px(8.0))
                .px(px(16.0))
                .py(px(4.0))
                .bg(ShellDeckColors::bg_sidebar())
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .child(
                    div()
                        .text_size(px(10.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ShellDeckColors::text_muted())
                        .child(t!("scripts.requires").to_string()),
                );

            for dep in &script.dependencies {
                bar = bar.child(
                    div()
                        .text_size(px(10.0))
                        .px(px(4.0))
                        .py(px(1.0))
                        .rounded(px(3.0))
                        .bg(ShellDeckColors::badge_bg())
                        .text_color(ShellDeckColors::text_muted())
                        .child(dep.name.clone()),
                );
            }

            Some(bar)
        } else {
            None
        };

        // Script body area - either inline editor or readonly
        let body_area = if is_inline_editing {
            let cursor_pos = Some(self.inline_buffer.cursor_line_col());

            div()
                .flex()
                .flex_col()
                .flex_grow()
                .min_h_0()
                // Inline editing toolbar
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .px(px(16.0))
                        .py(px(6.0))
                        .bg(ShellDeckColors::primary().opacity(0.08))
                        .border_b_1()
                        .border_color(ShellDeckColors::primary().opacity(0.2))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(ShellDeckColors::primary())
                                .font_weight(FontWeight::MEDIUM)
                                .child(t!("scripts.editing_body").to_string()),
                        )
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.0))
                                .child(
                                    div()
                                        .id("inline-cancel-btn")
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.cancel_inline_edit();
                                            cx.notify();
                                        }))
                                        .child(
                                            Button::new(
                                                "inline-cancel",
                                                t!("scripts.cancel_esc").to_string(),
                                            )
                                            .variant(ButtonVariant::Ghost),
                                        ),
                                )
                                .child(
                                    div()
                                        .id("inline-save-btn")
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.save_inline_edit(cx);
                                            cx.notify();
                                        }))
                                        .child(
                                            Button::new(
                                                "inline-save",
                                                t!("scripts.save").to_string(),
                                            )
                                            .variant(ButtonVariant::Default),
                                        ),
                                ),
                        ),
                )
                // Editable code block
                .child(
                    div()
                        .flex_grow()
                        .min_h_0()
                        .p(px(16.0))
                        .id("script-body")
                        .overflow_y_scroll()
                        .child(
                            div()
                                .id("script-body-code")
                                .w_full()
                                .rounded(px(8.0))
                                .bg(ShellDeckColors::terminal_bg())
                                .border_1()
                                .border_color(ShellDeckColors::primary())
                                .font_family("JetBrains Mono")
                                .overflow_hidden()
                                .cursor_text()
                                .child(render_code_block_with_language(
                                    self.inline_buffer.text(),
                                    cursor_pos,
                                    true,
                                    lang,
                                )),
                        ),
                )
        } else {
            div().flex().flex_col().flex_grow().min_h_0().child(
                div()
                    .flex_grow()
                    .min_h_0()
                    .p(px(16.0))
                    .id("script-body")
                    .overflow_y_scroll()
                    .child(
                        div()
                            .id("script-body-code")
                            .w_full()
                            .rounded(px(8.0))
                            .bg(ShellDeckColors::terminal_bg())
                            .border_1()
                            .border_color(ShellDeckColors::border())
                            .font_family("JetBrains Mono")
                            .overflow_hidden()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                if event.click_count() >= 2 {
                                    this.start_inline_edit(script_id);
                                    this.focus_handle.focus(window);
                                    cx.notify();
                                }
                            }))
                            .child(render_code_block_with_language(
                                &script.body,
                                None,
                                false,
                                lang,
                            )),
                    ),
            )
        };

        let mut editor = div()
            .relative()
            .flex()
            .flex_col()
            .flex_grow()
            .h_full()
            .min_h_0()
            .overflow_hidden()
            // Script header
            .child(header);

        // Dependency bar
        if let Some(dep_bar) = dep_bar {
            editor = editor.child(dep_bar);
        }

        // Script body
        editor = editor.child(body_area);

        // Variables bar (between code and output)
        if let Some(var_bar) = self.render_variables_bar(script, cx) {
            editor = editor.child(var_bar);
        }

        // Output panel
        editor = editor.child(self.render_output_panel(cx));

        editor
    }

    fn build_run_target_menu_items(
        &self,
        script: &Script,
        entity: Entity<Self>,
    ) -> Vec<PopoverMenuItem> {
        let mut items = Vec::new();

        let script_local = {
            let mut s = script.clone();
            s.target = ScriptTarget::Local;
            s
        };
        items.push(
            PopoverMenuItem::new("run-target-local", t!("scripts.target.local").to_string())
                .icon("terminal")
                .on_click({
                    let entity = entity.clone();
                    move |_, cx| {
                        entity.update(cx, |this, cx| {
                            this.close_run_target_menu(cx);
                            cx.emit(ScriptEvent::RunScript(script_local.clone()));
                        });
                    }
                }),
        );

        if self.run_target_connections.is_empty() {
            items.push(
                PopoverMenuItem::new("run-target-none", t!("scripts.no_connections").to_string())
                    .disabled(true),
            );
        } else {
            for (conn_id, conn_name) in &self.run_target_connections {
                let script_remote = {
                    let mut s = script.clone();
                    s.target = ScriptTarget::Remote(*conn_id);
                    s
                };
                let item_id = format!("run-target-{conn_id}");
                let label = conn_name.clone();
                items.push(
                    PopoverMenuItem::new(item_id, label)
                        .icon("server")
                        .on_click({
                            let entity = entity.clone();
                            move |_, cx| {
                                entity.update(cx, |this, cx| {
                                    this.close_run_target_menu(cx);
                                    cx.emit(ScriptEvent::RunScript(script_remote.clone()));
                                });
                            }
                        }),
                );
            }
        }

        items
    }

    fn render_run_target_popover(
        &self,
        script: &Script,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let items = self.build_run_target_menu_items(script, entity.clone());
        PopoverMenu::new(pos, items)
            .max_height(gpui::px(300.0))
            .on_close({
                let entity = entity.clone();
                move |_, cx| {
                    entity.update(cx, |this, cx| {
                        this.close_run_target_menu(cx);
                    });
                }
            })
    }

    fn render_variables_bar(&self, script: &Script, cx: &mut Context<Self>) -> Option<Div> {
        let resolved = script.resolved_variables();
        if resolved.is_empty() {
            return None;
        }

        let last_values = self.last_var_values.get(&script.id);
        let script_id = script.id;

        let mut pills = div().flex().flex_wrap().gap(px(6.0));

        for var in &resolved {
            let value = last_values.and_then(|m| m.get(&var.name)).cloned();

            let label = var.label.as_deref().unwrap_or(&var.name);
            let display = if let Some(ref val) = value {
                format!("{}: {}", label, val)
            } else if let Some(ref def) = var.default_value {
                format!("{}: {} (default)", label, def)
            } else {
                format!("{}: --", label)
            };

            let has_value = value.is_some();

            let pill = div()
                .px(px(8.0))
                .py(px(2.0))
                .rounded(px(4.0))
                .text_size(px(11.0))
                .font_family("JetBrains Mono")
                .bg(if has_value {
                    ShellDeckColors::primary().opacity(0.12)
                } else {
                    ShellDeckColors::bg_sidebar()
                })
                .text_color(if has_value {
                    ShellDeckColors::primary()
                } else {
                    ShellDeckColors::text_muted()
                })
                .child(display);

            pills = pills.child(pill);
        }

        // "Edit" button to re-open the variable prompt
        let script_for_run = script.clone();
        let edit_btn = div()
            .id(SharedString::from(format!("var-edit-{}", script_id)))
            .px(px(8.0))
            .py(px(2.0))
            .rounded(px(4.0))
            .text_size(px(11.0))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .text_color(ShellDeckColors::primary())
            .hover(|el| el.bg(ShellDeckColors::primary().opacity(0.1)))
            .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                // Re-run with the variable prompt by emitting RunScript
                cx.emit(ScriptEvent::RunScript(script_for_run.clone()));
            }))
            .child(t!("scripts.run_with_vars").to_string());

        let bar = div()
            .flex()
            .items_center()
            .gap(px(8.0))
            .px(px(12.0))
            .py(px(6.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .bg(ShellDeckColors::bg_sidebar())
            .child(
                div()
                    .text_size(px(10.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .flex_shrink_0()
                    .child(t!("scripts.vars").to_string()),
            )
            .child(pills)
            .child(edit_btn);

        Some(bar)
    }

    fn render_output_panel(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut output_label = div().flex().items_center().gap(px(8.0)).child(
            div()
                .text_size(px(11.0))
                .font_weight(FontWeight::BOLD)
                .text_color(ShellDeckColors::text_muted())
                .child(t!("scripts.output").to_string()),
        );

        if self.is_running() {
            output_label = output_label.child(
                div()
                    .w(px(6.0))
                    .h(px(6.0))
                    .rounded_full()
                    .bg(ShellDeckColors::success()),
            );
        }

        let fix_button = self
            .ai_generation_enabled
            .then(|| self.selected_failed_execution())
            .flatten()
            .and(self.selected_script)
            .map(|script_id| {
                Button::new(
                    "fix-failed-script-ai",
                    t!("ai.workflow.script_fix").to_string(),
                )
                .variant(ButtonVariant::Ai)
                .size(ButtonSize::Sm)
                .icon(IconSource::from("sparkles"))
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(ScriptEvent::FixWithAi(script_id));
                }))
            });

        let panel = div()
            .relative()
            .flex()
            .flex_col()
            .h(px(self.output_panel_height))
            .flex_shrink_0()
            .border_t_1()
            .border_color(ShellDeckColors::border())
            // Resize handle
            .child(
                div()
                    .id("output-resize-handle")
                    .absolute()
                    .top(px(-3.0))
                    .left_0()
                    .right_0()
                    .h(px(6.0))
                    .cursor_row_resize()
                    .hover(|el| el.bg(ShellDeckColors::primary().opacity(0.4)))
                    .when(self.output_resizing, |el| {
                        el.bg(ShellDeckColors::primary().opacity(0.6))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.output_resizing = true;
                            cx.notify();
                        }),
                    ),
            )
            // Output header
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(12.0))
                    .py(px(6.0))
                    .bg(ShellDeckColors::bg_sidebar())
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(output_label)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .children(fix_button)
                            .child(
                                div()
                                    .id("copy-output-btn")
                                    .cursor_pointer()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        let text = this.execution_output.join("\n");
                                        cx.write_to_clipboard(ClipboardItem::new_string(text));
                                    }))
                                    .child(t!("scripts.copy").to_string()),
                            )
                            .child(
                                div()
                                    .id("clear-output-btn")
                                    .cursor_pointer()
                                    .text_size(px(11.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.execution_output.clear();
                                        cx.emit(ScriptEvent::ClearOutput);
                                        cx.notify();
                                    }))
                                    .child(t!("scripts.clear").to_string()),
                            ),
                    ),
            )
            // Output content
            .child(
                div()
                    .flex_grow()
                    .p(px(8.0))
                    .bg(ShellDeckColors::terminal_bg())
                    .id("script-output")
                    .overflow_y_scroll()
                    .font_family("JetBrains Mono")
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_primary())
                    .children(
                        self.execution_output
                            .iter()
                            .map(|line| div().child(line.clone())),
                    ),
            );

        panel
    }

    fn render_empty_editor() -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .flex_grow()
            .h_full()
            .bg(ShellDeckColors::bg_primary())
            .gap(px(12.0))
            .child(
                div()
                    .text_size(px(14.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("scripts.select_hint").to_string()),
            )
    }
}

impl ScriptEditorView {
    /// Floating kebab (⋮) row-action menu: Run / Delete. Positioned at the
    /// click coords via `deferred(anchored())` so it escapes list clipping;
    /// dismisses on any click outside via `on_mouse_down_out`.
    fn render_kebab_menu(
        &self,
        script_id: Uuid,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let script = self.scripts.iter().find(|s| s.id == script_id)?;
        let title = script.name.clone();

        let run_accent = ShellDeckColors::success();
        let del_accent = ShellDeckColors::error();

        let panel = div()
            .id("scripts-kebab-panel")
            .occlude()
            .w(px(200.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(8.0))
            .shadow(
                vec![BoxShadow {
                    color: hsla(0.0, 0.0, 0.0, 0.35),
                    offset: point(gpui::px(0.0), gpui::px(4.0)),
                    blur_radius: gpui::px(16.0),
                    spread_radius: gpui::px(0.0),
                    inset: false,
                }]
                .into(),
            )
            .p(px(4.0))
            .flex()
            .flex_col()
            .gap(px(1.0))
            .on_mouse_down_out(cx.listener(|this, _e, _window, cx| {
                this.kebab_menu = None;
                cx.notify();
            }))
            // Header
            .child(
                div()
                    .px(px(10.0))
                    .py(px(6.0))
                    .text_size(px(11.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .child(title),
            )
            .child(div().h(px(1.0)).my(px(2.0)).bg(ShellDeckColors::border()))
            // Run
            .child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "scripts-kebab-run-{script_id}"
                    ))))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(5.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_primary())
                    .cursor_pointer()
                    .hover(move |el| el.bg(run_accent.opacity(0.12)).text_color(run_accent))
                    .child(t!("scripts.run").to_string())
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                        cx.stop_propagation();
                        this.kebab_menu = None;
                        cx.emit(ScriptEvent::RunScriptById(script_id));
                    })),
            )
            .child(div().h(px(1.0)).my(px(2.0)).bg(ShellDeckColors::border()))
            // Delete
            .child(
                div()
                    .id(ElementId::from(SharedString::from(format!(
                        "scripts-kebab-del-{script_id}"
                    ))))
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(5.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::error())
                    .cursor_pointer()
                    .hover(move |el| el.bg(del_accent.opacity(0.12)).text_color(del_accent))
                    .child(t!("scripts.delete").to_string())
                    .on_click(cx.listener(move |this, _e: &ClickEvent, _window, cx| {
                        cx.stop_propagation();
                        this.kebab_menu = None;
                        cx.emit(ScriptEvent::DeleteScript(script_id));
                    })),
            );

        Some(
            deferred(
                anchored()
                    .position(pos + point(gpui::px(0.0), gpui::px(4.0)))
                    .anchor(gpui::Corner::TopLeft)
                    .snap_to_window_with_margin(gpui::px(8.0))
                    .child(panel),
            )
            .with_priority(2)
            .into_any_element(),
        )
    }

    fn render_ai_actions_menu(
        &self,
        script_id: Uuid,
        pos: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let entity = cx.entity();
        let explain_entity = entity.clone();
        let review_entity = entity.clone();
        let items = vec![
            PopoverMenuItem::new(
                "script-ai-explain",
                t!("ai.workflow.script_explain").to_string(),
            )
            .icon("info")
            .on_click(move |_, cx| {
                explain_entity.update(cx, |this, cx| {
                    this.ai_actions_menu = None;
                    cx.emit(ScriptEvent::ExplainWithAi(script_id));
                    cx.notify();
                });
            }),
            PopoverMenuItem::new(
                "script-ai-review",
                t!("ai.workflow.script_review").to_string(),
            )
            .icon("shield-check")
            .on_click(move |_, cx| {
                review_entity.update(cx, |this, cx| {
                    this.ai_actions_menu = None;
                    cx.emit(ScriptEvent::ReviewWithAi(script_id));
                    cx.notify();
                });
            }),
        ];

        PopoverMenu::new(pos, items).on_close(move |_, cx| {
            entity.update(cx, |this, cx| {
                this.ai_actions_menu = None;
                cx.notify();
            });
        })
    }
}

impl Render for ScriptEditorView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.selected().cloned();
        let is_editing = self.inline_editing;

        let mut container = div()
            .id("script-editor-root")
            .flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, _window, cx| {
                if is_editing {
                    this.handle_inline_key_down(event, cx);
                } else {
                    this.handle_search_key_down(event, cx);
                }
            }))
            .child(self.render_script_list(cx));

        if let Some(ref script) = selected {
            container = container.child(self.render_editor(script, cx));
        } else {
            container = container.child(Self::render_empty_editor());
        }

        // Kebab (⋮) menu overlay for the currently-open row.
        if let Some((script_id, pos)) = self.kebab_menu {
            if let Some(menu) = self.render_kebab_menu(script_id, pos, cx) {
                container = container.child(menu);
            }
        }

        if let Some((script_id, pos)) = self.ai_actions_menu {
            container = container.child(self.render_ai_actions_menu(script_id, pos, cx));
        }

        if self.run_target_menu_open && !self.is_running() {
            if let (Some(script), Some(pos)) = (selected.as_ref(), self.run_target_menu_position())
            {
                container = container.child(self.render_run_target_popover(script, pos, cx));
            }
        }

        container
    }
}
