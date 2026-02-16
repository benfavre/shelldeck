use std::collections::HashMap;

use gpui::prelude::*;
use gpui::*;

use shelldeck_core::models::script::{Script, ScriptVariable};

use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum VariablePromptEvent {
    Run(Script, HashMap<String, String>),
    Cancel,
}

impl EventEmitter<VariablePromptEvent> for VariablePrompt {}

pub struct VariablePrompt {
    script: Script,
    variables: Vec<ScriptVariable>,
    values: Vec<String>,
    active_field: usize,
    focus_handle: FocusHandle,
}

impl VariablePrompt {
    pub fn new(script: Script, variables: Vec<ScriptVariable>, cx: &mut Context<Self>) -> Self {
        let values: Vec<String> = variables
            .iter()
            .map(|v| v.default_value.clone().unwrap_or_default())
            .collect();
        Self {
            script,
            variables,
            values,
            active_field: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    fn display_label(&self, var: &ScriptVariable) -> String {
        if let Some(ref label) = var.label {
            label.clone()
        } else {
            // Title-case the variable name: "project_path" -> "Project Path"
            var.name
                .split('_')
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().to_string() + c.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        }
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        let mods = &event.keystroke.modifiers;

        match key {
            "escape" => {
                cx.emit(VariablePromptEvent::Cancel);
                return;
            }
            "enter" => {
                if !mods.shift {
                    self.submit(cx);
                    return;
                }
            }
            "tab" => {
                if mods.shift {
                    if self.active_field > 0 {
                        self.active_field -= 1;
                    } else {
                        self.active_field = self.variables.len().saturating_sub(1);
                    }
                } else if !self.variables.is_empty() {
                    self.active_field = (self.active_field + 1) % self.variables.len();
                }
                cx.notify();
                return;
            }
            "backspace" => {
                if self.active_field < self.values.len() {
                    self.values[self.active_field].pop();
                    cx.notify();
                }
                return;
            }
            _ => {}
        }

        // Ctrl+V paste
        if key == "v" && mods.secondary() {
            if let Some(item) = cx.read_from_clipboard() {
                if let Some(text) = item.text() {
                    if self.active_field < self.values.len() {
                        self.values[self.active_field].push_str(&text);
                        cx.notify();
                    }
                }
            }
            return;
        }

        // Ctrl+A select all (clear field)
        if key == "a" && mods.secondary() {
            if self.active_field < self.values.len() {
                self.values[self.active_field].clear();
                cx.notify();
            }
            return;
        }

        // Printable characters
        if let Some(ref kc) = event.keystroke.key_char {
            if !mods.control && !mods.alt {
                if self.active_field < self.values.len() {
                    self.values[self.active_field].push_str(kc);
                    cx.notify();
                }
                return;
            }
        }

        if key.len() == 1 && !mods.control && !mods.alt
            && self.active_field < self.values.len() {
                self.values[self.active_field].push_str(key);
                cx.notify();
            }
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let mut map = HashMap::new();
        for (i, var) in self.variables.iter().enumerate() {
            let value = self.values.get(i).cloned().unwrap_or_default();
            if !value.is_empty() {
                map.insert(var.name.clone(), value);
            }
        }
        cx.emit(VariablePromptEvent::Run(self.script.clone(), map));
    }
}

impl Render for VariablePrompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focus_handle.focus(window);

        let field_count = self.variables.len();

        // Build variable fields
        let mut fields = div().flex().flex_col().gap(px(12.0));

        for (idx, var) in self.variables.iter().enumerate() {
            let label_text = self.display_label(var);
            let is_active = idx == self.active_field;
            let value = self.values.get(idx).cloned().unwrap_or_default();

            let display_value: SharedString = if value.is_empty() {
                SharedString::from(format!("{{{{{}}}}}",  var.name))
            } else {
                SharedString::from(value)
            };
            let value_is_empty = self.values.get(idx).map(|v| v.is_empty()).unwrap_or(true);

            let mut input = div()
                .id(SharedString::from(format!("var-input-{}", idx)))
                .px(px(10.0))
                .py(px(7.0))
                .rounded(px(6.0))
                .border_1()
                .text_size(px(13.0))
                .w_full()
                .cursor_text()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.active_field = idx;
                    cx.notify();
                }));

            if is_active {
                input = input
                    .border_color(ShellDeckColors::primary())
                    .bg(ShellDeckColors::bg_primary());
            } else {
                input = input
                    .border_color(ShellDeckColors::border())
                    .bg(ShellDeckColors::bg_primary());
            }

            if value_is_empty {
                input = input
                    .text_color(ShellDeckColors::text_muted())
                    .child(display_value);
            } else {
                input = input
                    .text_color(ShellDeckColors::text_primary())
                    .child(display_value);
            }

            // Cursor indicator for active field
            if is_active {
                // The cursor is shown via a blinking border effect
            }

            let mut field = div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(ShellDeckColors::text_primary())
                        .child(label_text),
                )
                .child(input);

            // Description help text
            if let Some(ref desc) = var.description {
                field = field.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(desc.clone()),
                );
            }

            fields = fields.child(field);
        }

        // Footer with hint + buttons
        let footer = div()
            .flex()
            .items_center()
            .justify_between()
            .px(px(20.0))
            .py(px(12.0))
            .border_t_1()
            .border_color(ShellDeckColors::border())
            .child(
                div()
                    .text_size(px(11.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(format!(
                        "Tab to switch fields ({}/{}) | Enter to run | Esc to cancel",
                        self.active_field + 1,
                        field_count
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap(px(8.0))
                    .child(
                        div()
                            .id("var-cancel-btn")
                            .px(px(14.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .cursor_pointer()
                            .bg(ShellDeckColors::bg_surface())
                            .text_color(ShellDeckColors::text_primary())
                            .hover(|el| el.bg(ShellDeckColors::hover_bg()))
                            .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                cx.emit(VariablePromptEvent::Cancel);
                            }))
                            .child("Cancel"),
                    )
                    .child(
                        div()
                            .id("var-run-btn")
                            .px(px(14.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .text_size(px(13.0))
                            .font_weight(FontWeight::MEDIUM)
                            .cursor_pointer()
                            .bg(ShellDeckColors::primary())
                            .text_color(gpui::white())
                            .hover(|el| el.opacity(0.9))
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.submit(cx);
                            }))
                            .child("Run"),
                    ),
            );

        // Main overlay
        div()
            .id("variable-prompt-overlay")
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
                    .w(px(480.0))
                    .max_h(px(520.0))
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
                            .py(px(12.0))
                            .border_b_1()
                            .border_color(ShellDeckColors::border())
                            .child(
                                div()
                                    .text_size(px(16.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(format!("Run: {}", self.script.name)),
                            )
                            .child(
                                div()
                                    .id("close-variable-prompt")
                                    .cursor_pointer()
                                    .text_size(px(16.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                        cx.emit(VariablePromptEvent::Cancel);
                                    }))
                                    .child("x"),
                            ),
                    )
                    // Body with fields
                    .child(
                        div()
                            .id("variable-prompt-body")
                            .flex()
                            .flex_col()
                            .flex_grow()
                            .overflow_y_scroll()
                            .px(px(20.0))
                            .py(px(16.0))
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .pb(px(12.0))
                                    .child("Fill in the template variables before running this script."),
                            )
                            .child(fields),
                    )
                    // Footer
                    .child(footer),
            )
    }
}
