use std::collections::HashMap;

use adabraka_ui::prelude::*;
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::models::execution::ExecutionRecord;
use shelldeck_core::models::script::{Script, ScriptCategory, ScriptLanguage, ScriptTarget};
use uuid::Uuid;

use crate::editor_buffer::EditorBuffer;
use crate::syntax::highlight::render_code_block_with_language;
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
    DeleteScript(Uuid),
    ImportTemplate(String),
    RunScriptById(Uuid),
}

impl EventEmitter<ScriptEvent> for ScriptEditorView {}

pub struct ScriptEditorView {
    pub scripts: Vec<Script>,
    pub selected_script: Option<Uuid>,
    pub execution_output: Vec<String>,
    pub running_script_id: Option<Uuid>,
    pub history: Vec<ExecutionRecord>,
    // Run target picker state
    run_target_open: bool,
    run_target_connections: Vec<(Uuid, String)>,
    // Output panel state
    output_panel_height: f32,
    output_resizing: bool,
    // Inline editing state
    inline_editing: bool,
    inline_buffer: EditorBuffer,
    inline_script_id: Option<Uuid>,
    focus_handle: FocusHandle,
    // New: filtering state
    selected_category: Option<ScriptCategory>,
    search_query: String,
    show_favorites_only: bool,
    // Template browser toggle (handled in workspace)
    pub template_browser_open: bool,
    // Last-used variable values per script (script_id -> name -> value)
    pub last_var_values: HashMap<Uuid, HashMap<String, String>>,
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
            run_target_open: false,
            run_target_connections: Vec::new(),
            output_panel_height: 200.0,
            output_resizing: false,
            inline_editing: false,
            inline_buffer: EditorBuffer::new(),
            inline_script_id: None,
            focus_handle: cx.focus_handle(),
            selected_category: None,
            search_query: String::new(),
            show_favorites_only: false,
            template_browser_open: false,
            last_var_values: HashMap::new(),
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

    fn render_language_badge(lang: &ScriptLanguage) -> Div {
        let (r, g, b) = lang.badge_color();
        let color = gpui::hsla(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0);
        div()
            .text_size(px(9.0))
            .px(px(4.0))
            .py(px(1.0))
            .rounded(px(3.0))
            .bg(color.opacity(0.15))
            .text_color(color)
            .font_weight(FontWeight::SEMIBOLD)
            .child(lang.badge().to_string())
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
                        .child("Scripts"),
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
                                .child("Templates"),
                        )
                        // Add script button
                        .child(
                            div()
                                .id("add-script-btn")
                                .cursor_pointer()
                                .text_size(px(16.0))
                                .text_color(ShellDeckColors::text_muted())
                                .hover(|el| el.text_color(ShellDeckColors::primary()))
                                .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                    cx.emit(ScriptEvent::AddScript);
                                }))
                                .child("+"),
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
                                .child("Search scripts...")
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

            tabs = tabs.child(all_tab.child("All"));

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

                tabs = tabs.child(tab.child(cat.label()));
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
                                "No scripts yet"
                            } else {
                                "No matching scripts"
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child("Click + to create or browse templates"),
                    ),
            );
        }

        for script in filtered {
            let script_id = script.id;
            let is_selected = self.selected_script == Some(script.id);
            let target_label = match &script.target {
                ScriptTarget::Local => "Local",
                ScriptTarget::Remote(_) => "Remote",
                ScriptTarget::AskOnRun => "Ask",
            };
            let is_favorite = script.is_favorite;
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
                // Favorite star
                .child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "fav-{}",
                            script_id
                        ))))
                        .cursor_pointer()
                        .text_size(px(12.0))
                        .text_color(if is_favorite {
                            ShellDeckColors::warning()
                        } else {
                            ShellDeckColors::text_muted()
                        })
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(ScriptEvent::ToggleFavorite(script_id));
                        }))
                        .child(if is_favorite { "*" } else { "" }),
                );

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
                .child(div().flex_grow())
                // Quick run button (visible on hover via group)
                .child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "run-{}",
                            script_id
                        ))))
                        .cursor_pointer()
                        .text_size(px(10.0))
                        .px(px(4.0))
                        .py(px(1.0))
                        .rounded(px(3.0))
                        .bg(ShellDeckColors::success().opacity(0.1))
                        .text_color(ShellDeckColors::success())
                        .opacity(0.0)
                        .group_hover("script-item", |el| el.opacity(1.0))
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(ScriptEvent::RunScriptById(script_id));
                        }))
                        .child("Run"),
                )
                // Delete button (visible on hover via group)
                .child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "del-{}",
                            script_id
                        ))))
                        .cursor_pointer()
                        .text_size(px(10.0))
                        .px(px(4.0))
                        .py(px(1.0))
                        .rounded(px(3.0))
                        .text_color(ShellDeckColors::error())
                        .opacity(0.0)
                        .group_hover("script-item", |el| el.opacity(1.0))
                        .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                            cx.emit(ScriptEvent::DeleteScript(script_id));
                        }))
                        .child("x"),
                );

            list = list.child(item_el.child(row1).child(row2));
        }

        list
    }

    fn render_editor(&self, script: &Script, cx: &mut Context<Self>) -> impl IntoElement {
        let script_clone = script.clone();
        let script_for_dropdown = script.clone();
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
            div().flex().gap(px(4.0)).child(
                div()
                    .id("stop-script-btn")
                    .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                        cx.emit(ScriptEvent::StopScript);
                    }))
                    .child(Button::new("stop-script", "Stop").variant(ButtonVariant::Destructive)),
            )
        } else {
            div()
                .flex()
                .gap(px(0.0))
                .child(
                    div()
                        .id("run-script-btn")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            if !this.is_running() {
                                if matches!(script_clone.target, ScriptTarget::AskOnRun) {
                                    this.run_target_open = !this.run_target_open;
                                    cx.notify();
                                } else {
                                    cx.emit(ScriptEvent::RunScript(script_clone.clone()));
                                }
                            }
                        }))
                        .child(Button::new("run-script", "Run").variant(ButtonVariant::Default)),
                )
                .child(
                    div()
                        .id("run-target-dropdown-btn")
                        .cursor_pointer()
                        .px(px(6.0))
                        .py(px(5.0))
                        .rounded_r(px(6.0))
                        .bg(ShellDeckColors::primary().opacity(0.1))
                        .border_1()
                        .border_color(ShellDeckColors::primary().opacity(0.3))
                        .text_size(px(10.0))
                        .text_color(ShellDeckColors::primary())
                        .hover(|el| el.bg(ShellDeckColors::primary().opacity(0.2)))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.run_target_open = !this.run_target_open;
                            cx.notify();
                        }))
                        .child("v"),
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
                .child(Button::new("cancel-edit-script", "Cancel").variant(ButtonVariant::Ghost))
        } else {
            div()
                .id("edit-script-btn")
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.start_inline_edit(script_id);
                    this.focus_handle.focus(window);
                    cx.notify();
                }))
                .child(Button::new("edit-script", "Edit").variant(ButtonVariant::Ghost))
        };

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
                        .child("Requires:"),
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
                                .child("Editing script body"),
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
                                            Button::new("inline-cancel", "Cancel (Esc)")
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
                                            Button::new("inline-save", "Save (Ctrl+S)")
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

        // Run target picker — rendered last so it paints on top
        if self.run_target_open && !self.is_running() {
            editor = editor.child(self.render_run_target_picker(&script_for_dropdown, cx));
        }

        editor
    }

    fn render_run_target_picker(
        &self,
        script: &Script,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut picker = div()
            .absolute()
            .top(px(48.0))
            .right(px(16.0))
            .w(px(220.0))
            .bg(ShellDeckColors::bg_surface())
            .border_1()
            .border_color(ShellDeckColors::border())
            .rounded(px(8.0))
            .shadow_lg()
            .overflow_hidden()
            .flex()
            .flex_col();

        // "Local" option
        let script_local = {
            let mut s = script.clone();
            s.target = ScriptTarget::Local;
            s
        };
        picker = picker.child(
            div()
                .id("run-target-local")
                .px(px(12.0))
                .py(px(8.0))
                .cursor_pointer()
                .text_size(px(13.0))
                .text_color(ShellDeckColors::text_primary())
                .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.run_target_open = false;
                    cx.emit(ScriptEvent::RunScript(script_local.clone()));
                    cx.notify();
                }))
                .child("Local"),
        );

        // Separator
        picker = picker.child(div().h(px(1.0)).bg(ShellDeckColors::border()));

        if self.run_target_connections.is_empty() {
            picker = picker.child(
                div()
                    .px(px(12.0))
                    .py(px(8.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("No connections available"),
            );
        } else {
            for (i, (conn_id, conn_name)) in self.run_target_connections.iter().enumerate() {
                let script_remote = {
                    let mut s = script.clone();
                    s.target = ScriptTarget::Remote(*conn_id);
                    s
                };
                picker = picker.child(
                    div()
                        .id(ElementId::from(SharedString::from(format!(
                            "run-target-{}",
                            i
                        ))))
                        .px(px(12.0))
                        .py(px(8.0))
                        .cursor_pointer()
                        .text_size(px(13.0))
                        .text_color(ShellDeckColors::text_primary())
                        .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.run_target_open = false;
                            cx.emit(ScriptEvent::RunScript(script_remote.clone()));
                            cx.notify();
                        }))
                        .child(conn_name.clone()),
                );
            }
        }

        picker
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
            .child("Run with variables...");

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
                    .child("VARS"),
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
                .child("OUTPUT"),
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
                                    .child("Copy"),
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
                                    .child("Clear"),
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
                    .child("Select a script to view or edit"),
            )
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

        if let Some(script) = selected {
            container = container.child(self.render_editor(&script, cx));
        } else {
            container = container.child(Self::render_empty_editor());
        }

        container
    }
}
