use crate::scale::px;
use adabraka_ui::components::combobox::Combobox;
use adabraka_ui::components::input::{Input, InputSize, InputState};
use gpui::prelude::*;
use gpui::*;

use shelldeck_core::models::script::{Script, ScriptCategory, ScriptLanguage, ScriptTarget};
use uuid::Uuid;

use crate::connection_combobox::{build_connection_combobox, connection_idx_for_id};
use crate::editor_buffer::EditorBuffer;
use crate::icons::{script_category_chip, script_language_chip};
use crate::syntax::highlight::render_code_block_with_language;
use crate::theme::ShellDeckColors;
use crate::t;

#[derive(Debug, Clone, Copy)]
enum ValidationError {
    NameRequired,
    NoConnections,
}

fn script_form_error(err: ValidationError) -> String {
    match err {
        ValidationError::NameRequired => t!("script_form.error.name_required").to_string(),
        ValidationError::NoConnections => t!("script_form.error.no_connections").to_string(),
    }
}

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
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
    name_state: Entity<InputState>,
    description_state: Entity<InputState>,
    language: ScriptLanguage,
    category: ScriptCategory,
    body: EditorBuffer,
    target: ScriptTarget,
    error: Option<String>,
    error_field: Option<FormField>,
    /// Non-text active field (Body / Target / Language / Category / Connection).
    /// Text fields (Name / Description) own their own focus via `InputState`.
    active_field: Option<FormField>,
    focus_handle: FocusHandle,
    needs_focus: bool,
    connection_combobox: Entity<Combobox<Uuid>>,
}

fn new_input_state_sf(cx: &mut Context<ScriptForm>, initial: &str) -> Entity<InputState> {
    let initial = initial.to_string();
    cx.new(|cx| {
        let mut s = InputState::new(cx);
        if !initial.is_empty() {
            s.content = initial.into();
        }
        s
    })
}

impl ScriptForm {
    fn init_connection_combobox(
        connections: &[(Uuid, String, String)],
        selected_idx: usize,
        cx: &mut Context<Self>,
    ) -> Entity<Combobox<Uuid>> {
        let parent = cx.entity();
        let placeholder = if connections.is_empty() {
            t!("script_form.connection.none").to_string()
        } else {
            t!("script_form.connection.select").to_string()
        };
        let (_state, combobox) = build_connection_combobox(
            connections,
            selected_idx,
            &placeholder,
            move |id, _window, cx| {
                parent.update(cx, |form, cx| {
                    if let Some(idx) = connection_idx_for_id(&form.connections, *id) {
                        form.selected_connection_idx = idx;
                        if matches!(form.target, ScriptTarget::Remote(_)) {
                            form.target = ScriptTarget::Remote(*id);
                        }
                        form.error = None;
                        form.error_field = None;
                    }
                    cx.notify();
                });
            },
            cx,
        );
        combobox
    }

    pub fn new(connections: Vec<(Uuid, String, String)>, cx: &mut Context<Self>) -> Self {
        let connection_combobox = Self::init_connection_combobox(&connections, 0, cx);
        Self {
            editing_id: None,
            connections,
            selected_connection_idx: 0,
            name_state: new_input_state_sf(cx, ""),
            description_state: new_input_state_sf(cx, ""),
            language: ScriptLanguage::Shell,
            category: ScriptCategory::Uncategorized,
            body: EditorBuffer::new(),
            target: ScriptTarget::Local,
            error: None,
            error_field: None,
            active_field: None,
            focus_handle: cx.focus_handle(),
            needs_focus: true,
            connection_combobox,
        }
    }

    pub fn from_script(
        script: &Script,
        connections: Vec<(Uuid, String, String)>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (target, selected_idx) = match &script.target {
            ScriptTarget::Remote(conn_id) => {
                let idx = connections
                    .iter()
                    .position(|(id, _, _)| id == conn_id)
                    .unwrap_or(0);
                (script.target.clone(), idx)
            }
            other => (other.clone(), 0),
        };
        let connection_combobox = Self::init_connection_combobox(&connections, selected_idx, cx);
        Self {
            editing_id: Some(script.id),
            connections,
            selected_connection_idx: selected_idx,
            name_state: new_input_state_sf(cx, &script.name),
            description_state: new_input_state_sf(cx, script.description.as_deref().unwrap_or("")),
            language: script.language.clone(),
            category: script.category,
            body: EditorBuffer::from_text(script.body.clone()),
            target,
            error: None,
            error_field: None,
            active_field: None,
            focus_handle: cx.focus_handle(),
            needs_focus: true,
            connection_combobox,
        }
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn field_value(state: &Entity<InputState>, cx: &Context<Self>) -> String {
        state.read(cx).content().to_string()
    }

    fn is_valid(&self, cx: &Context<Self>) -> bool {
        !Self::field_value(&self.name_state, cx).is_empty()
    }

    /// Text field editing lives inside the focused `Input` widget now — this
    /// helper only exists for legacy callers still asking for a mutable ref
    /// to a text buffer. Always returns None post-migration.
    fn active_field_mut(&mut self) -> Option<&mut String> {
        None
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
        // Delegate to body-specific handler
        if self.active_field == Some(FormField::Body) {
            self.handle_body_key_down(event, cx);
            return;
        }

        let key = event.keystroke.key.as_str();

        match key {
            "escape" => {
                cx.emit(ScriptFormEvent::Cancel);
            }
            "enter" => {
                self.try_save(cx);
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

    pub fn try_save(&mut self, cx: &mut Context<Self>) {
        match self.validate(cx) {
            Ok(script) => {
                cx.emit(ScriptFormEvent::Save(script));
            }
            Err(err) => {
                self.error_field = Some(match err {
                    ValidationError::NameRequired => FormField::Name,
                    ValidationError::NoConnections => FormField::Connection,
                });
                self.error = Some(script_form_error(err));
                cx.notify();
            }
        }
    }

    fn validate(&self, cx: &Context<Self>) -> Result<Script, ValidationError> {
        let name = Self::field_value(&self.name_state, cx);
        let description = Self::field_value(&self.description_state, cx);
        if name.is_empty() {
            return Err(ValidationError::NameRequired);
        }
        let target = match &self.target {
            ScriptTarget::Remote(_) => {
                if self.connections.is_empty() {
                    return Err(ValidationError::NoConnections);
                }
                ScriptTarget::Remote(self.connections[self.selected_connection_idx].0)
            }
            other => other.clone(),
        };

        let mut script = Script::new_with_language(
            name,
            self.body.text().to_string(),
            target,
            self.language.clone(),
            self.category,
        );
        if !description.is_empty() {
            script.description = Some(description);
        }

        if let Some(id) = self.editing_id {
            script.id = id;
        }

        Ok(script)
    }

    fn render_text_field(
        &self,
        field: Option<FormField>,
        label: impl Into<SharedString>,
        state: &Entity<InputState>,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let has_error = field.is_some() && field == self.error_field;
        let placeholder = placeholder.into();
        let input = Input::new(state)
            .size(InputSize::Sm)
            .placeholder(placeholder)
            .error(has_error)
            .on_change({
                let entity = cx.entity();
                move |_v, cx| {
                    entity.update(cx, |this, cx| {
                        if this.error.is_some() {
                            this.error = None;
                            this.error_field = None;
                            cx.notify();
                        }
                    });
                }
            })
            .on_enter({
                let entity = cx.entity();
                move |_v, cx| {
                    entity.update(cx, |this, cx| this.try_save(cx));
                }
            });

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_muted())
                    .child(label.into()),
            )
            .child(input)
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
                    .child(t!("script_form.body.placeholder").to_string()),
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
            input_box = input_box.child(render_code_block_with_language(
                self.body.text(),
                cursor_pos,
                is_active,
                &self.language,
            ));
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
                            .child(t!("script_form.field.body").to_string()),
                    )
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(ShellDeckColors::text_muted())
                            .child(t!("script_form.body.hint").to_string()),
                    ),
            )
            .child(input_box)
    }

    fn render_target_chips(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_active = self.active_field == Some(FormField::Target);
        let options: Vec<(ScriptTarget, String)> = vec![
            (
                ScriptTarget::Local,
                t!("scripts.target.local").to_string(),
            ),
            (
                ScriptTarget::Remote(Uuid::nil()),
                t!("scripts.target.remote").to_string(),
            ),
            (
                ScriptTarget::AskOnRun,
                t!("script_form.target.ask_on_run").to_string(),
            ),
        ];

        let mut chips = div().flex().gap(px(6.0));

        for (target, label) in options {
            let selected = matches!(
                (&self.target, &target),
                (ScriptTarget::Local, ScriptTarget::Local)
                    | (ScriptTarget::Remote(_), ScriptTarget::Remote(_))
                    | (ScriptTarget::AskOnRun, ScriptTarget::AskOnRun)
            );

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

            chips = chips.child(chip.child(label.clone()));
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
                    .child(t!("script_form.field.target").to_string()),
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
            let badge_color = gpui::hsla(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0);

            let mut chip = div()
                .id(ElementId::from(SharedString::from(format!(
                    "sf-lang-{}",
                    lang.label()
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
                // Neutral surface + brand ring — not brand-tinted bg (Bun yellow /
                // Docker blue on same-hue wash kills contrast). No chip-level
                // text_color: it cascades onto SVG fills via GPUI.
                chip = chip
                    .bg(ShellDeckColors::selected_bg())
                    .border_1()
                    .border_color(badge_color);
            } else {
                chip = chip
                    .bg(ShellDeckColors::bg_primary())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .hover(|el| el.border_color(ShellDeckColors::text_muted()));
            }

            let label_color = if selected {
                ShellDeckColors::text_primary()
            } else {
                ShellDeckColors::text_muted()
            };
            chips = chips.child(chip.child(script_language_chip(lang.clone(), label_color)));
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
                    .child(t!("script_form.field.language").to_string()),
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
                    "sf-cat-{}",
                    cat.label()
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

            let icon_color = if selected {
                ShellDeckColors::primary()
            } else {
                ShellDeckColors::text_muted()
            };
            chips = chips.child(chip.child(script_category_chip(*cat, icon_color)));
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
                    .child(t!("script_form.field.category").to_string()),
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

    fn render_connection_picker(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let has_error = self.error_field == Some(FormField::Connection);

        div()
            .flex()
            .flex_col()
            .gap(px(4.0))
            .child(
                div()
                    .text_size(px(12.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("script_form.field.connection").to_string()),
            )
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .when(has_error, |el| {
                        el.rounded(px(6.0))
                            .border_1()
                            .border_color(ShellDeckColors::error())
                    })
                    .child(self.connection_combobox.clone()),
            )
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
            // Name
            .child(self.render_text_field(
                Some(FormField::Name),
                t!("script_form.field.name").to_string(),
                &self.name_state,
                t!("script_form.field.name_placeholder").to_string(),
                cx,
            ))
            .child(self.render_text_field(
                None,
                t!("script_form.field.description").to_string(),
                &self.description_state,
                t!("script_form.field.description_placeholder").to_string(),
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
            // Legacy hand-rolled modal — must cap height + scroll (see .agents/overflow.md).
            // TODO: migrate to adabraka Dialog (see .agents/ui-components.md).
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
                    .max_h(px(580.0))
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
                            .flex_shrink_0()
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
                                    .child(if self.editing_id.is_some() {
                                        t!("script_form.title.edit").to_string()
                                    } else {
                                        t!("script_form.title.new").to_string()
                                    }),
                            )
                            .child(
                                div()
                                    .id("close-sf-form-btn")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_color(ShellDeckColors::text_muted())
                                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                                    .child(
                                        svg()
                                            .path("icons/lucide/x.svg")
                                            .size(px(14.0))
                                            .text_color(ShellDeckColors::text_muted()),
                                    )
                                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                        cx.emit(ScriptFormEvent::Cancel);
                                    })),
                            ),
                    )
                    // Scrollable form body
                    .child(
                        div()
                            .id("script-form-body")
                            .flex()
                            .flex_col()
                            .flex_grow()
                            .min_h(px(0.0))
                            .overflow_y_scroll()
                            .p(px(20.0))
                            .child(form_fields),
                    )
                    // Footer
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
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
                                        adabraka_ui::prelude::Button::new(
                                            "cancel",
                                            t!("scripts.cancel").to_string(),
                                        )
                                            .variant(adabraka_ui::prelude::ButtonVariant::Ghost),
                                    ),
                            )
                            .child({
                                let valid = self.is_valid(cx);
                                let btn_label = if self.editing_id.is_some() {
                                    t!("script_form.save.edit").to_string()
                                } else {
                                    t!("script_form.save.create").to_string()
                                };
                                let mut save_btn = div()
                                    .id("sf-save-btn")
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.try_save(cx);
                                    }))
                                    .child(
                                        adabraka_ui::prelude::Button::new("save", btn_label)
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
