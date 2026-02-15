use gpui::prelude::*;
use gpui::*;

use crate::theme::ShellDeckColors;

actions!(shelldeck, [ToggleCommandPalette]);

/// A registered action with a display name and shortcut hint.
#[derive(Debug)]
pub struct PaletteAction {
    pub name: String,
    pub shortcut: Option<String>,
    pub action: Box<dyn Action>,
}

impl Clone for PaletteAction {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            shortcut: self.shortcut.clone(),
            action: self.action.boxed_clone(),
        }
    }
}

impl PaletteAction {
    pub fn new(name: &str, shortcut: Option<&str>, action: Box<dyn Action>) -> Self {
        Self {
            name: name.to_string(),
            shortcut: shortcut.map(String::from),
            action,
        }
    }
}

/// Events emitted by the command palette.
pub enum CommandPaletteEvent {
    ActionSelected(Box<dyn Action>),
    Dismissed,
}

impl std::fmt::Debug for CommandPaletteEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ActionSelected(_) => write!(f, "ActionSelected(...)"),
            Self::Dismissed => write!(f, "Dismissed"),
        }
    }
}

impl EventEmitter<CommandPaletteEvent> for CommandPalette {}

pub struct CommandPalette {
    pub visible: bool,
    pub query: String,
    pub actions: Vec<PaletteAction>,
    pub filtered: Vec<usize>,
    pub selected_index: usize,
    pub focus_handle: FocusHandle,
}

impl CommandPalette {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            visible: false,
            query: String::new(),
            actions: Vec::new(),
            filtered: Vec::new(),
            selected_index: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn toggle(&mut self, window: &mut Window) {
        self.visible = !self.visible;
        if self.visible {
            self.query.clear();
            self.selected_index = 0;
            self.update_filter();
            self.focus_handle.focus(window);
        }
    }

    pub fn dismiss(&mut self) {
        self.visible = false;
        self.query.clear();
    }

    pub fn set_actions(&mut self, actions: Vec<PaletteAction>) {
        self.actions = actions;
        self.update_filter();
    }

    fn update_filter(&mut self) {
        let query_lower = self.query.to_lowercase();
        if query_lower.is_empty() {
            self.filtered = (0..self.actions.len()).collect();
        } else {
            self.filtered = self.actions.iter().enumerate()
                .filter(|(_, a)| fuzzy_match(&a.name, &query_lower))
                .map(|(i, _)| i)
                .collect();
        }
        self.selected_index = 0;
    }

    pub fn select_next(&mut self) {
        if !self.filtered.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.filtered.len();
        }
    }

    pub fn select_prev(&mut self) {
        if !self.filtered.is_empty() {
            self.selected_index = if self.selected_index == 0 {
                self.filtered.len() - 1
            } else {
                self.selected_index - 1
            };
        }
    }

    pub fn selected_action(&self) -> Option<&PaletteAction> {
        self.filtered.get(self.selected_index)
            .and_then(|&i| self.actions.get(i))
    }

    /// Confirm the currently selected action.
    fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(action) = self.selected_action() {
            cx.emit(CommandPaletteEvent::ActionSelected(action.action.boxed_clone()));
        }
        self.dismiss();
        cx.notify();
    }

    /// Handle key events for the palette input.
    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        match key {
            "escape" => {
                cx.emit(CommandPaletteEvent::Dismissed);
                self.dismiss();
                cx.notify();
            }
            "enter" => {
                self.confirm(cx);
            }
            "up" => {
                self.select_prev();
                cx.notify();
            }
            "down" => {
                self.select_next();
                cx.notify();
            }
            "backspace" => {
                self.query.pop();
                self.update_filter();
                cx.notify();
            }
            _ => {
                if let Some(ref kc) = event.keystroke.key_char {
                    if !event.keystroke.modifiers.control && !event.keystroke.modifiers.alt {
                        self.query.push_str(kc);
                        self.update_filter();
                        cx.notify();
                    }
                } else if key.len() == 1
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.alt
                {
                    self.query.push_str(key);
                    self.update_filter();
                    cx.notify();
                }
            }
        }
    }
}

/// Simple fuzzy subsequence match.
pub fn fuzzy_match(haystack: &str, needle: &str) -> bool {
    let haystack = haystack.to_lowercase();
    let mut haystack_chars = haystack.chars();
    for needle_char in needle.chars() {
        loop {
            match haystack_chars.next() {
                Some(h) if h == needle_char => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

impl Render for CommandPalette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("palette-hidden");
        }

        let mut list = div()
            .flex()
            .flex_col()
            .max_h(px(400.0))
            .overflow_hidden();

        if self.filtered.is_empty() && !self.query.is_empty() {
            list = list.child(
                div()
                    .px(px(14.0))
                    .py(px(12.0))
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child("No matching commands"),
            );
        }

        for (fi, &action_idx) in self.filtered.iter().enumerate() {
            if fi > 20 {
                break; // Limit display
            }
            let action = &self.actions[action_idx];
            let is_selected = fi == self.selected_index;

            let mut item = div()
                .id(ElementId::from(SharedString::from(format!("palette-{}", fi))))
                .flex()
                .items_center()
                .justify_between()
                .px(px(14.0))
                .py(px(8.0))
                .mx(px(4.0))
                .rounded(px(6.0))
                .cursor_pointer()
                .text_size(px(14.0));

            if is_selected {
                item = item
                    .bg(ShellDeckColors::primary().opacity(0.15))
                    .text_color(ShellDeckColors::primary());
            }

            item = item.hover(|el| el.bg(ShellDeckColors::hover_bg()));

            let name = action.name.clone();
            let action_clone = action.action.boxed_clone();
            item = item
                .child(
                    div()
                        .text_color(ShellDeckColors::text_primary())
                        .child(name),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.emit(CommandPaletteEvent::ActionSelected(action_clone.boxed_clone()));
                    this.dismiss();
                    cx.notify();
                }));

            if let Some(shortcut) = &action.shortcut {
                item = item.child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(shortcut.clone()),
                );
            }

            list = list.child(item);
        }

        // Outer container with focus tracking and key handling
        div()
            .id("command-palette")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _window, cx| {
                this.handle_key_down(event, cx);
            }))
            .absolute()
            .top(px(0.0))
            .left(px(0.0))
            .right(px(0.0))
            .bottom(px(0.0))
            // Semi-transparent backdrop
            .bg(ShellDeckColors::backdrop())
            .flex()
            .justify_center()
            .pt(px(80.0))
            .child(
                div()
                    .w(px(520.0))
                    .max_h(px(460.0))
                    .bg(ShellDeckColors::bg_surface())
                    .border_1()
                    .border_color(ShellDeckColors::border())
                    .rounded(px(10.0))
                    .shadow_xl()
                    .overflow_hidden()
                    .flex()
                    .flex_col()
                    // Query input area
                    .child(
                        div()
                            .px(px(14.0))
                            .py(px(12.0))
                            .border_b_1()
                            .border_color(ShellDeckColors::border())
                            .text_size(px(15.0))
                            .child(if self.query.is_empty() {
                                div()
                                    .text_color(ShellDeckColors::text_muted())
                                    .child("Type a command...")
                            } else {
                                div()
                                    .text_color(ShellDeckColors::text_primary())
                                    .flex()
                                    .child(self.query.clone())
                                    .child(
                                        div()
                                            .w(px(1.0))
                                            .h(px(16.0))
                                            .bg(ShellDeckColors::primary()),
                                    )
                            }),
                    )
                    // Results list
                    .child(list),
            )
    }
}
