use gpui::prelude::*;
use gpui::*;

use shelldeck_core::models::script::{Script, ScriptCategory, ScriptLanguage, ScriptTarget};
use uuid::Uuid;

use crate::command_palette::fuzzy_match;
use crate::editor_buffer::EditorBuffer;
use crate::syntax::highlight::render_code_block_with_language;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
pub enum ScriptFormEvent {
    Save(Script),
    Cancel,
}

impl EventEmitter<ScriptFormEvent> for ScriptForm {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FormField {
    Name,
    Description,
    Language,
    Category,
    Body,
    Target,
    Connection,
}

impl FormField {
    const ALL: &[FormField] = &[
        FormField::Name,
        FormField::Description,
        FormField::Language,
        FormField::Category,
        FormField::Body,
        FormField::Target,
        FormField::Connection,
    ];

    fn next(self) -> FormField {
        let idx = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }
}

pub struct ScriptForm {
    editing_id: Option<Uuid>,
    connections: Vec<(Uuid, String, String)>,
    selected_connection_idx: usize,
    name: String,
    description: String,
    language: ScriptLanguage,
    category: ScriptCategory,
    body: EditorBuffer,
    target: ScriptTarget,
    error: Option<String>,
    error_field: Option<FormField>,
    active_field: Option<FormField>,
    focus_handle: FocusHandle,
    needs_focus: bool,
    // Dropdown state
    dropdown_open: bool,
    dropdown_query: String,
    dropdown_filtered: Vec<usize>,
    dropdown_selected: usize,
}

impl ScriptForm {
    pub fn new(connections: Vec<(Uuid, String, String)>, cx: &mut Context<Self>) -> Self {
        let dropdown_filtered = (0..connections.len()).collect();
        Self {
            editing_id: None,
            connections,
            selected_connection_idx: 0,
            name: String::new(),
            description: String::new(),
            language: ScriptLanguage::Shell,
            category: ScriptCategory::Uncategorized,
            body: EditorBuffer::new(),
            target: ScriptTarget::Local,
            error: None,
            error_field: None,
            active_field: Some(FormField::Name),
            focus_handle: cx.focus_handle(),
            needs_focus: true,
            dropdown_open: false,
            dropdown_query: String::new(),
            dropdown_filtered,
            dropdown_selected: 0,
        }
    }

    pub fn from_script(script: &Script, connections: Vec<(Uuid, String, String)>, cx: &mut Context<Self>) -> Self {
        let (target, selected_idx) = match &script.target {
            ScriptTarget::Remote(conn_id) => {
                let idx = connections.iter().position(|(id, _, _)| id == conn_id).unwrap_or(0);
                (script.target.clone(), idx)
            }
            other => (other.clone(), 0),
        };
        let dropdown_filtered = (0..connections.len()).collect();
        Self {
            editing_id: Some(script.id),
            connections,
            selected_connection_idx: selected_idx,
            name: script.name.clone(),
            description: script.description.clone().unwrap_or_default(),
            language: script.language.clone(),
            category: script.category,
            body: EditorBuffer::from_text(script.body.clone()),
            target,
            error: None,
            error_field: None,
            active_field: Some(FormField::Name),
            focus_handle: cx.focus_handle(),
            needs_focus: true,
            dropdown_open: false,
            dropdown_query: String::new(),
            dropdown_filtered,
            dropdown_selected: 0,
        }
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn is_valid(&self) -> bool {
        !self.name.is_empty()
    }

    /// Returns mutable reference to the active text field (Name/Description only).
    /// Body uses EditorBuffer and is handled separately.
    fn active_field_mut(&mut self) -> Option<&mut String> {
        self.active_field.map(move |f| match f {
            FormField::Name => Some(&mut self.name),
            FormField::Description => Some(&mut self.description),
            FormField::Body => None,
            FormField::Target => None,
            FormField::Language => None,
            FormField::Category => None,
            FormField::Connection => None,
        })?
    }

    fn cycle_target(&mut self) {
        self.target = match &self.target {
            ScriptTarget::Local => {
                if self.connections.is_empty() {
                    ScriptTarget::AskOnRun
                } else {
                    ScriptTarget::Remote(self.connections[self.selected_connection_idx].0)
                }
            }
            ScriptTarget::Remote(_) => ScriptTarget::AskOnRun,
            ScriptTarget::AskOnRun => ScriptTarget::Local,
        };
    }

    fn update_dropdown_filter(&mut self) {
        let query_lower = self.dropdown_query.to_lowercase();
        if query_lower.is_empty() {
            self.dropdown_filtered = (0..self.connections.len()).collect();
        } else {
            self.dropdown_filtered = self.connections.iter().enumerate()
                .filter(|(_, (_, name, host))| {
                    fuzzy_match(name, &query_lower) || fuzzy_match(host, &query_lower)
                })
                .map(|(i, _)| i)
                .collect();
        }
        self.dropdown_selected = 0;
    }

    fn close_dropdown(&mut self) {
        self.dropdown_open = false;
        self.dropdown_query.clear();
        self.dropdown_filtered = (0..self.connections.len()).collect();
        self.dropdown_selected = 0;
    }

    /// Handle key events when the body editor field is active.
    fn handle_body_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;

        match key {
            "escape" => {
                cx.emit(ScriptFormEvent::Cancel);
                return;
            }
            "left" => {
                self.body.move_left();
                cx.notify();
                return;
            }
            "right" => {
                self.body.move_right();
                cx.notify();
                return;
            }
            "up" => {
                self.body.move_up();
                cx.notify();
                return;
            }
            "down" => {
                self.body.move_down();
                cx.notify();
                return;
            }
            "home" => {
                self.body.move_home();
                cx.notify();
                return;
            }
            "end" => {
                self.body.move_end();
                cx.notify();
                return;
            }
            "enter" => {
                self.body.insert_newline();
                self.error = None;
                self.error_field = None;
                cx.notify();
                return;
            }
            "backspace" => {
                self.body.backspace();
                self.error = None;
                self.error_field = None;
                cx.notify();
                return;
            }
            "delete" => {
                self.body.delete();
                self.error = None;
                self.error_field = None;
                cx.notify();
                return;
            }
            "tab" => {
                if mods.shift {
                    // Shift+Tab: cycle to next field
                    self.active_field = Some(FormField::Body.next());
                } else {
                    // Tab: insert spaces
                    self.body.insert_tab();
                    self.error = None;
                    self.error_field = None;
                }
                cx.notify();
                return;
            }
            _ => {}
        }

        // Ctrl+V / Cmd+V paste
        if key == "v" && mods.secondary() {
            if let Some(item) = cx.read_from_clipboard() {
                if let Some(text) = item.text() {
                    self.body.insert_str(&text);
                    self.error = None;
                    self.error_field = None;
                    cx.notify();
                }
            }
            return;
        }

        // Printable characters
        if let Some(ref kc) = event.keystroke.key_char {
            if !mods.control && !mods.alt {
                for c in kc.chars() {
                    self.body.insert_char(c);
                }
                self.error = None;
                self.error_field = None;
                cx.notify();
                return;
            }
        }

        // Single-char key fallback (for keys without key_char)
        if key.len() == 1 && !mods.control && !mods.alt {
            for c in key.chars() {
                self.body.insert_char(c);
            }
            self.error = None;
            self.error_field = None;
            cx.notify();
        }
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();

        // When dropdown is open, intercept all keys
        if self.dropdown_open {
            match key {
                "escape" => {
                    self.close_dropdown();
                    cx.notify();
                }
                "up" => {
                    if !self.dropdown_filtered.is_empty() && self.dropdown_selected > 0 {
                        self.dropdown_selected -= 1;
                    }
                    cx.notify();
                }
                "down" => {
                    if !self.dropdown_filtered.is_empty() {
                        self.dropdown_selected = (self.dropdown_selected + 1).min(self.dropdown_filtered.len() - 1);
                    }
                    cx.notify();
                }
                "enter" => {
                    if let Some(&conn_idx) = self.dropdown_filtered.get(self.dropdown_selected) {
                        self.selected_connection_idx = conn_idx;
                        if matches!(self.target, ScriptTarget::Remote(_)) {
                            self.target = ScriptTarget::Remote(self.connections[conn_idx].0);
                        }
                    }
                    self.close_dropdown();
                    cx.notify();
                }
                "backspace" => {
                    self.dropdown_query.pop();
                    self.update_dropdown_filter();
                    cx.notify();
                }
                _ => {
                    if let Some(ref kc) = event.keystroke.key_char {
                        if !event.keystroke.modifiers.control && !event.keystroke.modifiers.alt {
                            self.dropdown_query.push_str(kc);
                            self.update_dropdown_filter();
                            cx.notify();
                        }
                    } else if key.len() == 1
                        && !event.keystroke.modifiers.control
                        && !event.keystroke.modifiers.alt
                    {
                        self.dropdown_query.push_str(key);
                        self.update_dropdown_filter();
                        cx.notify();
                    }
                }
            }
            return;
        }

        // Delegate to body-specific handler
        if self.active_field == Some(FormField::Body) {
            self.handle_body_key_down(event, cx);
            return;
        }

        match key {
            "escape" => {
                cx.emit(ScriptFormEvent::Cancel);
            }
            "enter" => {
                if self.active_field == Some(FormField::Connection) {
                    self.dropdown_open = true;
                    self.dropdown_query.clear();
                    self.update_dropdown_filter();
                    cx.notify();
                } else {
                    self.try_save(cx);
                }
            }
            "tab" => {
                if let Some(field) = self.active_field {
                    self.active_field = Some(field.next());
                    cx.notify();
                }
            }
            "backspace" => {
                if let Some(field) = self.active_field_mut() {
                    field.pop();
                    self.error = None;
                    self.error_field = None;
                    cx.notify();
                }
            }
            " " => {
                if self.active_field == Some(FormField::Target) {
                    self.cycle_target();
                    self.error = None;
                    self.error_field = None;
                    cx.notify();
                    return;
                }
                if self.active_field == Some(FormField::Connection) {
                    self.dropdown_open = true;
                    self.dropdown_query.clear();
                    self.update_dropdown_filter();
                    cx.notify();
                    return;
                }
                if let Some(field) = self.active_field_mut() {
                    field.push(' ');
                    self.error = None;
                    self.error_field = None;
                    cx.notify();
                }
            }
            _ => {
                if let Some(ref kc) = event.keystroke.key_char {
                    if !event.keystroke.modifiers.control && !event.keystroke.modifiers.alt {
                        if let Some(field) = self.active_field_mut() {
                            field.push_str(kc);
                            self.error = None;
                            self.error_field = None;
                            cx.notify();
                        }
                    }
                } else if key.len() == 1
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                {
                    if let Some(field) = self.active_field_mut() {
                        field.push_str(key);
                        self.error = None;
                        self.error_field = None;
                        cx.notify();
                    }
                }
            }
        }
    }

    fn try_save(&mut self, cx: &mut Context<Self>) {
        match self.validate() {
            Ok(script) => {
                cx.emit(ScriptFormEvent::Save(script));
            }
            Err(msg) => {
                if msg.contains("name") {
                    self.error_field = Some(FormField::Name);
                } else if msg.contains("body") {
                    self.error_field = Some(FormField::Body);
                } else if msg.contains("connections") {
                    self.error_field = Some(FormField::Connection);
                }
                self.error = Some(msg);
                cx.notify();
            }
        }
    }

    fn validate(&self) -> Result<Script, String> {
        if self.name.is_empty() {
            return Err("Script name is required".to_string());
        }
        let target = match &self.target {
            ScriptTarget::Remote(_) => {
                if self.connections.is_empty() {
                    return Err("No connections available for remote target".to_string());
                }
                ScriptTarget::Remote(self.connections[self.selected_connection_idx].0)
            }
            other => other.clone(),
        };

        let mut script = Script::new_with_language(
            self.name.clone(),
            self.body.text().to_string(),
            target,
            self.language.clone(),
            self.category,
        );
        if !self.description.is_empty() {
            script.description = Some(self.description.clone());
        }

        if let Some(id) = self.editing_id {
            script.id = id;
        }

        Ok(script)
    }

    fn render_text_field(
        &self,
        field: FormField,
        label: &str,
        value: &str,
        placeholder: &str,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_active = self.active_field == Some(field);
        let has_error = self.error_field == Some(field);

        let mut input_box = div()
            .id(ElementId::from(SharedString::from(format!(
                "sf-field-{field:?}"
            ))))
            .w_full()
            .px(px(10.0))
            .py(px(6.0))
            .rounded(px(6.0))
            .bg(ShellDeckColors::bg_primary())
            .border_1()
            .text_size(px(13.0))
            .cursor_text()
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.active_field = Some(field);
                cx.notify();
            }));

        if has_error {
            input_box = input_box.border_color(ShellDeckColors::error());
        } else if is_active {
            input_box = input_box.border_color(ShellDeckColors::primary());
        } else {
            input_box = input_box.border_color(ShellDeckColors::border());
        }

        if value.is_empty() {
            input_box = input_box.child(
                div()
                    .text_color(ShellDeckColors::text_muted())
                    .child(placeholder.to_string()),
            );
        } else {
            let mut text_el = div()
                .text_color(ShellDeckColors::text_primary())
                .flex()
                .child(value.to_string());

            if is_active {
                text_el = text_el.child(
                    div()
                        .w(px(1.0))
                        .h(px(16.0))
                        .bg(ShellDeckColors::primary()),
                );
            }

            input_box = input_box.child(text_el);
        }

        if value.is_empty() && is_active {
            input_box = input_box.child(
                div()
                    .absolute()
                    .left(px(10.0))
                    .top(px(6.0))
                    .w(px(1.0))
                    .h(px(16.0))
                    .bg(ShellDeckColors::primary()),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_muted())
                    .child(label.to_string()),
            )
            .child(input_box)
    }

    fn render_body_field(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_active = self.active_field == Some(FormField::Body);
        let has_error = self.error_field == Some(FormField::Body);

        let cursor_pos = if is_active {
            Some(self.body.cursor_line_col())
        } else {
            None
        };

        let mut input_box = div()
            .id("sf-field-Body")
            .w_full()
            .rounded(px(6.0))
            .bg(ShellDeckColors::terminal_bg())
            .border_1()
            .font_family("JetBrains Mono")
            .min_h(px(120.0))
            .cursor_text()
            .overflow_hidden()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.active_field = Some(FormField::Body);
                cx.notify();
            }));

        if has_error {
            input_box = input_box.border_color(ShellDeckColors::error());
        } else if is_active {
            input_box = input_box.border_color(ShellDeckColors::primary());
        } else {
            input_box = input_box.border_color(ShellDeckColors::border());
        }

        if self.body.is_empty() {
            input_box = input_box.child(
                div()
                    .px(px(10.0))
                    .py(px(6.0))
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("echo 'hello world'"),
            );
            if is_active {
                input_box = input_box.child(
                    div()
                        .absolute()
                        .left(px(10.0))
                        .top(px(6.0))
                        .w(px(1.5))
                        .h(px(16.0))
                        .bg(ShellDeckColors::primary()),
                );
            }
        } else {
            input_box = input_box.child(render_code_block_with_language(self.body.text(), cursor_pos, is_active, &self.language));
        }

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(ShellDeckColors::text_muted())
                            .child("Script Body"),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child("Tab inserts spaces · Shift+Tab next field"),
                    ),
            )
            .child(input_box)
    }

    fn render_target_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_active = self.active_field == Some(FormField::Target);
        let options: Vec<(ScriptTarget, &str)> = vec![
            (ScriptTarget::Local, "Local"),
            (ScriptTarget::Remote(Uuid::nil()), "Remote"),
            (ScriptTarget::AskOnRun, "Ask on Run"),
        ];

        let mut chips = div().flex().gap(px(6.0));

        for (target, label) in options {
            let selected = match (&self.target, &target) {
                (ScriptTarget::Local, ScriptTarget::Local) => true,
                (ScriptTarget::Remote(_), ScriptTarget::Remote(_)) => true,
                (ScriptTarget::AskOnRun, ScriptTarget::AskOnRun) => true,
                _ => false,
            };

            let target_for_click = match &target {
                ScriptTarget::Remote(_) => {
                    if self.connections.is_empty() {
                        ScriptTarget::AskOnRun
                    } else {
                        ScriptTarget::Remote(self.connections[self.selected_connection_idx].0)
                    }
                }
                other => other.clone(),
            };

            let mut chip = div()
                .id(ElementId::from(SharedString::from(format!(
                    "sf-target-{label}"
                ))))
                .px(px(10.0))
                .py(px(4.0))
                .rounded(px(6.0))
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.target = target_for_click.clone();
                    this.active_field = Some(FormField::Target);
                    this.error = None;
                    cx.notify();
                }));

            if selected {
                chip = chip
                    .bg(ShellDeckColors::primary().opacity(0.2))
                    .text_color(ShellDeckColors::primary())
                    .border_1()
                    .border_color(ShellDeckColors::primary());
            } else {
                chip = chip
                    .bg(ShellDeckColors::bg_primary())
                    .text_color(ShellDeckColors::text_muted())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .hover(|el| el.border_color(ShellDeckColors::text_muted()));
            }

            chips = chips.child(chip.child(label));
        }

        let mut wrapper_border = ShellDeckColors::border();
        if is_active {
            wrapper_border = ShellDeckColors::primary();
        }

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_muted())
                    .child("Target"),
            )
            .child(
                div()
                    .w_full()
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(wrapper_border)
                    .child(chips),
            )
    }

    fn render_language_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_active = self.active_field == Some(FormField::Language);

        let mut chips = div().flex().flex_wrap().gap(px(4.0));

        for lang in ScriptLanguage::ALL {
            let selected = self.language == *lang;
            let lang_clone = lang.clone();
            let (r, g, b) = lang.badge_color();
            let badge_color = gpui::hsla(
                r as f32 / 255.0,
                g as f32 / 255.0,
                b as f32 / 255.0,
                1.0,
            );

            let mut chip = div()
                .id(ElementId::from(SharedString::from(format!(
                    "sf-lang-{}", lang.label()
                ))))
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.language = lang_clone.clone();
                    this.active_field = Some(FormField::Language);
                    this.error = None;
                    cx.notify();
                }));

            if selected {
                chip = chip
                    .bg(badge_color.opacity(0.2))
                    .text_color(badge_color)
                    .border_1()
                    .border_color(badge_color);
            } else {
                chip = chip
                    .bg(ShellDeckColors::bg_primary())
                    .text_color(ShellDeckColors::text_muted())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .hover(|el| el.border_color(ShellDeckColors::text_muted()));
            }

            chips = chips.child(chip.child(lang.label()));
        }

        let mut wrapper_border = ShellDeckColors::border();
        if is_active {
            wrapper_border = ShellDeckColors::primary();
        }

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_muted())
                    .child("Language"),
            )
            .child(
                div()
                    .w_full()
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(wrapper_border)
                    .child(chips),
            )
    }

    fn render_category_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_active = self.active_field == Some(FormField::Category);

        let mut chips = div().flex().flex_wrap().gap(px(4.0));

        for cat in ScriptCategory::ALL {
            let selected = self.category == *cat;

            let mut chip = div()
                .id(ElementId::from(SharedString::from(format!(
                    "sf-cat-{}", cat.label()
                ))))
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(6.0))
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.category = *cat;
                    this.active_field = Some(FormField::Category);
                    this.error = None;
                    cx.notify();
                }));

            if selected {
                chip = chip
                    .bg(ShellDeckColors::primary().opacity(0.2))
                    .text_color(ShellDeckColors::primary())
                    .border_1()
                    .border_color(ShellDeckColors::primary());
            } else {
                chip = chip
                    .bg(ShellDeckColors::bg_primary())
                    .text_color(ShellDeckColors::text_muted())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .hover(|el| el.border_color(ShellDeckColors::text_muted()));
            }

            chips = chips.child(chip.child(cat.label()));
        }

        let mut wrapper_border = ShellDeckColors::border();
        if is_active {
            wrapper_border = ShellDeckColors::primary();
        }

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_muted())
                    .child("Category"),
            )
            .child(
                div()
                    .w_full()
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(wrapper_border)
                    .child(chips),
            )
    }

    fn render_connection_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_active = self.active_field == Some(FormField::Connection);
        let conn_name = if self.connections.is_empty() {
            "(no connections)".to_string()
        } else {
            self.connections[self.selected_connection_idx].1.clone()
        };

        let mut border_color = ShellDeckColors::border();
        if is_active {
            border_color = ShellDeckColors::primary();
        }

        let mut wrapper = div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_muted())
                    .child("Connection (for Remote)"),
            )
            .child(
                div()
                    .id("sf-connection-picker")
                    .w_full()
                    .px(px(10.0))
                    .py(px(6.0))
                    .rounded(px(6.0))
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(border_color)
                    .cursor_pointer()
                    .flex()
                    .items_center()
                    .justify_between()
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.active_field = Some(FormField::Connection);
                        if this.dropdown_open {
                            this.close_dropdown();
                        } else {
                            this.dropdown_open = true;
                            this.dropdown_query.clear();
                            this.update_dropdown_filter();
                        }
                        this.error = None;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .text_size(px(13.0))
                            .text_color(ShellDeckColors::text_primary())
                            .child(conn_name),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(if self.dropdown_open { "^" } else { "v" }),
                    ),
            );

        // Dropdown list
        if self.dropdown_open {
            let search_area = div()
                .px(px(8.0))
                .py(px(6.0))
                .border_b_1()
                .border_color(ShellDeckColors::border())
                .text_size(px(12.0))
                .child(if self.dropdown_query.is_empty() {
                    div()
                        .text_color(ShellDeckColors::text_muted())
                        .flex()
                        .child("Type to filter...")
                        .child(
                            div()
                                .w(px(1.0))
                                .h(px(14.0))
                                .bg(ShellDeckColors::primary()),
                        )
                } else {
                    div()
                        .text_color(ShellDeckColors::text_primary())
                        .flex()
                        .child(self.dropdown_query.clone())
                        .child(
                            div()
                                .w(px(1.0))
                                .h(px(14.0))
                                .bg(ShellDeckColors::primary()),
                        )
                });

            let mut items_list = div()
                .id("sf-dropdown-list")
                .flex()
                .flex_col()
                .max_h(px(200.0))
                .overflow_y_scroll();

            if self.dropdown_filtered.is_empty() {
                items_list = items_list.child(
                    div()
                        .px(px(8.0))
                        .py(px(10.0))
                        .text_size(px(12.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child("No matching connections"),
                );
            } else {
                for (fi, &conn_idx) in self.dropdown_filtered.iter().enumerate() {
                    let (_, ref name, ref hostname) = self.connections[conn_idx];
                    let is_highlighted = fi == self.dropdown_selected;

                    let mut item = div()
                        .id(ElementId::from(SharedString::from(format!("sf-dd-{}", fi))))
                        .flex()
                        .flex_col()
                        .gap(px(1.0))
                        .px(px(8.0))
                        .py(px(5.0))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.selected_connection_idx = conn_idx;
                            if matches!(this.target, ScriptTarget::Remote(_)) {
                                this.target = ScriptTarget::Remote(this.connections[conn_idx].0);
                            }
                            this.close_dropdown();
                            cx.notify();
                        }));

                    if is_highlighted {
                        item = item.bg(ShellDeckColors::primary().opacity(0.15));
                    } else {
                        item = item.hover(|el| el.bg(ShellDeckColors::hover_bg()));
                    }

                    item = item
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(ShellDeckColors::text_primary())
                                .child(name.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(ShellDeckColors::text_muted())
                                .child(hostname.clone()),
                        );

                    items_list = items_list.child(item);
                }
            }

            wrapper = wrapper.child(
                div()
                    .w_full()
                    .mt(px(2.0))
                    .bg(ShellDeckColors::bg_surface())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .rounded(px(6.0))
                    .overflow_hidden()
                    .shadow_md()
                    .child(search_area)
                    .child(items_list),
            );
        }

        wrapper
    }
}

impl Render for ScriptForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.needs_focus {
            self.needs_focus = false;
            self.focus_handle.focus(window);
        }
        let show_connection = matches!(self.target, ScriptTarget::Remote(_));

        let mut form_fields = div()
            .flex()
            .flex_col()
            .gap(px(12.0))
            .p(px(20.0))
            // Name
            .child(self.render_text_field(
                FormField::Name,
                "Script Name",
                &self.name.clone(),
                "My Script",
                cx,
            ))
            // Description
            .child(self.render_text_field(
                FormField::Description,
                "Description (optional)",
                &self.description.clone(),
                "What does this script do?",
                cx,
            ))
            // Language chips
            .child(self.render_language_chips(cx))
            // Category chips
            .child(self.render_category_chips(cx))
            // Body (multi-line with syntax highlighting)
            .child(self.render_body_field(cx))
            // Target chips
            .child(self.render_target_chips(cx));

        // Connection picker (only shown when target is Remote)
        if show_connection {
            form_fields = form_fields.child(self.render_connection_picker(cx));
        }

        // Error message
        if let Some(ref error) = self.error {
            form_fields = form_fields.child(
                div()
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::error())
                    .child(error.clone()),
            );
        }

        div()
            .id("script-form-overlay")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                this.handle_key_down(event, cx);
            }))
            .occlude()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .bg(ShellDeckColors::backdrop())
            .flex()
            .justify_center()
            .items_center()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(px(500.0))
                    .bg(ShellDeckColors::bg_surface())
                    .rounded(px(12.0))
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .shadow_xl()
                    .overflow_hidden()
                    // Header
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px(px(20.0))
                            .py(px(16.0))
                            .border_b_1()
                            .border_color(ShellDeckColors::border())
                            .child(
                                div()
                                    .text_size(px(16.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(if self.editing_id.is_some() { "Edit Script" } else { "New Script" }),
                            )
                            .child(
                                div()
                                    .id("close-sf-form-btn")
                                    .cursor_pointer()
                                    .text_size(px(16.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                                    .child("x")
                                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                        cx.emit(ScriptFormEvent::Cancel);
                                    })),
                            ),
                    )
                    // Form fields
                    .child(form_fields)
                    // Footer
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap(px(8.0))
                            .px(px(20.0))
                            .py(px(16.0))
                            .border_t_1()
                            .border_color(ShellDeckColors::border())
                            .child(
                                div()
                                    .id("sf-cancel-btn")
                                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                        cx.emit(ScriptFormEvent::Cancel);
                                    }))
                                    .child(
                                        adabraka_ui::prelude::Button::new("cancel", "Cancel")
                                            .variant(adabraka_ui::prelude::ButtonVariant::Ghost),
                                    ),
                            )
                            .child({
                                let valid = self.is_valid();
                                let btn_label = if self.editing_id.is_some() { "Save Script" } else { "Create Script" };
                                let mut save_btn = div()
                                    .id("sf-save-btn")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.try_save(cx);
                                    }))
                                    .child(
                                        adabraka_ui::prelude::Button::new(
                                            "save",
                                            btn_label,
                                        )
                                        .variant(adabraka_ui::prelude::ButtonVariant::Default),
                                    );
                                if valid {
                                    save_btn = save_btn.cursor_pointer();
                                } else {
                                    save_btn = save_btn.opacity(0.5);
                                }
                                save_btn
                            }),
                    ),
            )
    }
}
