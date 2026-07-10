use std::collections::HashMap;

use crate::scale::px;
use adabraka_ui::components::input::{Input, InputSize, InputState};
use gpui::prelude::*;
use gpui::*;

use shelldeck_core::models::script::{Script, ScriptVariable};

use crate::t;
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
    /// One `InputState` per variable, initialised with the variable's default.
    /// Cursor / selection / undo are owned by each widget individually.
    states: Vec<Entity<InputState>>,
    focus_handle: FocusHandle,
}

impl VariablePrompt {
    pub fn new(script: Script, variables: Vec<ScriptVariable>, cx: &mut Context<Self>) -> Self {
        let states: Vec<Entity<InputState>> = variables
            .iter()
            .map(|v| {
                let initial = v.default_value.clone().unwrap_or_default();
                cx.new(|cx| {
                    let mut s = InputState::new(cx);
                    if !initial.is_empty() {
                        s.content = initial.into();
                    }
                    s
                })
            })
            .collect();
        Self {
            script,
            variables,
            states,
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

    /// Escape only — typing / Tab / Enter are handled by the focused `Input`
    /// (Enter fires per-field `on_enter` which submits the whole prompt).
    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            cx.emit(VariablePromptEvent::Cancel);
        }
    }

    pub fn submit(&mut self, cx: &mut Context<Self>) {
        let mut map = HashMap::new();
        for (i, var) in self.variables.iter().enumerate() {
            let value = self
                .states
                .get(i)
                .map(|s| s.read(cx).content().to_string())
                .unwrap_or_default();
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

        // Build variable fields — each is a real `Input`. Enter submits the
        // whole prompt (matches the pre-migration behavior).
        let mut fields = div().flex().flex_col().gap(px(12.0));

        for (idx, var) in self.variables.iter().enumerate() {
            let label_text = self.display_label(var);
            let placeholder = SharedString::from(format!("{{{{{}}}}}", var.name));
            let Some(state) = self.states.get(idx) else {
                continue;
            };

            let input = Input::new(state)
                .size(InputSize::Sm)
                .placeholder(placeholder)
                .on_enter({
                    let entity = cx.entity();
                    move |_v, cx| {
                        entity.update(cx, |this, cx| this.submit(cx));
                    }
                });

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
                    .child(t!("variable_prompt.hint", count = field_count).to_string()),
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
                            .child(t!("variable_prompt.cancel").to_string()),
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
                            .child(t!("variable_prompt.run").to_string()),
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
                                    .child(
                                        t!(
                                            "variable_prompt.title",
                                            name = self.script.name.as_str()
                                        )
                                        .to_string(),
                                    ),
                            )
                            .child(
                                div()
                                    .id("close-variable-prompt")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_color(ShellDeckColors::text_muted())
                                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                        cx.emit(VariablePromptEvent::Cancel);
                                    }))
                                    .child(
                                        svg()
                                            .path("icons/lucide/x.svg")
                                            .size(px(14.0))
                                            .text_color(ShellDeckColors::text_muted()),
                                    ),
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
                                    .child(t!("variable_prompt.description").to_string()),
                            )
                            .child(fields),
                    )
                    // Footer
                    .child(footer),
            )
    }
}
