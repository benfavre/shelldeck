use crate::icons::lucide_icon;
use crate::scale::px;
use adabraka_ui::components::input::{Input, InputSize, InputState};
use gpui::prelude::*;
use gpui::*;
use shelldeck_core::config::app_config::ThemePreference;
use shelldeck_core::config::cloud_account::AppMode;

use crate::theme::ShellDeckColors;
use crate::t;

actions!(shelldeck, [ToggleCommandPalette]);

/// Apply a terminal color theme by name. Carried as data so a single action
/// type can drive every built-in theme entry in the command palette.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = shelldeck, no_json)]
pub struct ApplyTerminalTheme {
    pub name: String,
}

/// Apply an application (UI) theme. Used by the command palette's theme entries
/// so they can be previewed live as the selection moves and committed on enter.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = shelldeck, no_json)]
pub struct ApplyAppTheme {
    pub pref: ThemePreference,
}

/// Open an Inklura Manage area (by path) for the active site in the browser.
/// Carried as data so one action type drives every area entry in the palette.
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = shelldeck, no_json)]
pub struct OpenManageArea {
    pub path: String,
}

/// Switch the app mode (super-admins only; a no-op otherwise).
#[derive(Clone, PartialEq, Debug, Action)]
#[action(namespace = shelldeck, no_json)]
pub struct SetAppMode {
    pub mode: AppMode,
}

/// A registered action with a display name and shortcut hint.
#[derive(Debug)]
pub struct PaletteAction {
    pub name: String,
    pub shortcut: Option<String>,
    pub icon: &'static str,
    pub action: Box<dyn Action>,
}

impl Clone for PaletteAction {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            shortcut: self.shortcut.clone(),
            icon: self.icon,
            action: self.action.boxed_clone(),
        }
    }
}

impl PaletteAction {
    pub fn new(
        name: impl Into<String>,
        shortcut: Option<&str>,
        icon: &'static str,
        action: Box<dyn Action>,
    ) -> Self {
        Self {
            name: name.into(),
            shortcut: shortcut.map(String::from),
            icon,
            action,
        }
    }
}

/// Events emitted by the command palette.
pub enum CommandPaletteEvent {
    ActionSelected(Box<dyn Action>),
    /// The highlighted entry changed; carries the now-selected action so the
    /// workspace can live-preview it (e.g. app themes) before it is committed.
    SelectionPreviewed(Box<dyn Action>),
    Dismissed,
}

impl std::fmt::Debug for CommandPaletteEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ActionSelected(_) => write!(f, "ActionSelected(...)"),
            Self::SelectionPreviewed(_) => write!(f, "SelectionPreviewed(...)"),
            Self::Dismissed => write!(f, "Dismissed"),
        }
    }
}

impl EventEmitter<CommandPaletteEvent> for CommandPalette {}

pub struct CommandPalette {
    pub visible: bool,
    /// Real adabraka `Input` state — owns the cursor / selection / undo. The
    /// `query` string below is kept in sync via `on_change` for the filter
    /// helpers that need `&str` access.
    query_state: Entity<InputState>,
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
            query_state: cx.new(InputState::new),
            query: String::new(),
            actions: Vec::new(),
            filtered: Vec::new(),
            selected_index: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    /// Empty the `Input` buffer without needing a `Window` — `set_value`
    /// requires one and we don't always have it. We clear the public
    /// `content` field directly and let the widget re-read on next paint.
    fn reset_input(&self, cx: &mut Context<Self>) {
        self.query_state.update(cx, |s, cx| {
            s.content = "".into();
            cx.notify();
        });
    }

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.visible = !self.visible;
        if self.visible {
            self.reset_input(cx);
            self.query.clear();
            self.selected_index = 0;
            self.update_filter();
            // Focus the `Input` widget so typing goes straight into it;
            // Up/Down/Escape bubble to the palette root's `on_key_down`.
            let input_focus = self.query_state.read(cx).focus_handle(cx);
            input_focus.focus(window);
        }
    }

    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.reset_input(cx);
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
            self.filtered = self
                .actions
                .iter()
                .enumerate()
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
        self.filtered
            .get(self.selected_index)
            .and_then(|&i| self.actions.get(i))
    }

    /// Emit a preview event for the currently-highlighted action so the
    /// workspace can apply a live, uncommitted preview (used for app themes).
    fn emit_selection_preview(&self, cx: &mut Context<Self>) {
        if let Some(action) = self.selected_action() {
            cx.emit(CommandPaletteEvent::SelectionPreviewed(
                action.action.boxed_clone(),
            ));
        }
    }

    /// Confirm the currently selected action.
    fn confirm(&mut self, cx: &mut Context<Self>) {
        if let Some(action) = self.selected_action() {
            cx.emit(CommandPaletteEvent::ActionSelected(
                action.action.boxed_clone(),
            ));
        }
        self.dismiss(cx);
        cx.notify();
    }

    /// Non-text keys only — typing is handled inside the focused `Input`
    /// widget (which also fires `on_enter` for Enter). We intercept the
    /// list-navigation keys (Up/Down) and Escape.
    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => {
                cx.emit(CommandPaletteEvent::Dismissed);
                self.dismiss(cx);
                cx.notify();
            }
            "up" => {
                self.select_prev();
                self.emit_selection_preview(cx);
                cx.notify();
            }
            "down" => {
                self.select_next();
                self.emit_selection_preview(cx);
                cx.notify();
            }
            _ => {}
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

/// Lucide slug for a palette row — stable per action, independent of locale.
fn palette_icon_for(action: &PaletteAction) -> &'static str {
    action.icon
}

impl Render for CommandPalette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.visible {
            return div().id("palette-hidden");
        }

        let mut list = div().flex().flex_col().max_h(px(400.0)).overflow_hidden();

        if self.filtered.is_empty() && !self.query.is_empty() {
            list = list.child(
                div()
                    .px(px(14.0))
                    .py(px(12.0))
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("palette.no_match").to_string()),
            );
        }

        for (fi, &action_idx) in self.filtered.iter().enumerate() {
            if fi > 20 {
                break; // Limit display
            }
            let action = &self.actions[action_idx];
            let is_selected = fi == self.selected_index;
            let icon = palette_icon_for(action);
            let label_color = if is_selected {
                ShellDeckColors::primary()
            } else {
                ShellDeckColors::text_primary()
            };
            let icon_color = if is_selected {
                ShellDeckColors::primary()
            } else {
                ShellDeckColors::text_muted()
            };

            let mut item = div()
                .id(ElementId::from(SharedString::from(format!(
                    "palette-{}",
                    fi
                ))))
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

            // Hovering an entry highlights it and previews it live.
            item = item.on_hover(cx.listener(move |this, hovered: &bool, _window, cx| {
                if *hovered && this.selected_index != fi {
                    this.selected_index = fi;
                    this.emit_selection_preview(cx);
                    cx.notify();
                }
            }));

            let name = action.name.clone();
            let action_clone = action.action.boxed_clone();
            item = item
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .child(lucide_icon(icon, 14.0, icon_color))
                        .child(
                            div()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .text_color(label_color)
                                .child(name),
                        ),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.emit(CommandPaletteEvent::ActionSelected(
                        action_clone.boxed_clone(),
                    ));
                    this.dismiss(cx);
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
                    // Query input — real `Input` widget with cursor / undo /
                    // selection. `on_change` mirrors the value back into
                    // `self.query` for the fuzzy-match helper; `on_enter`
                    // confirms the currently-selected action.
                    .child(
                        div()
                            .px(px(8.0))
                            .py(px(8.0))
                            .border_b_1()
                            .border_color(ShellDeckColors::border())
                            .child(
                                Input::new(&self.query_state)
                                    .size(InputSize::Md)
                                    .placeholder(t!("palette.placeholder").to_string())
                                    .prefix(lucide_icon(
                                        "search",
                                        14.0,
                                        ShellDeckColors::text_muted(),
                                    ))
                                    .on_change({
                                        let entity = cx.entity();
                                        move |value, cx| {
                                            entity.update(cx, |this, cx| {
                                                this.query = value.to_string();
                                                this.update_filter();
                                                this.emit_selection_preview(cx);
                                                cx.notify();
                                            });
                                        }
                                    })
                                    .on_enter({
                                        let entity = cx.entity();
                                        move |_v, cx| {
                                            entity.update(cx, |this, cx| this.confirm(cx));
                                        }
                                    }),
                            ),
                    )
                    // Results list
                    .child(list),
            )
    }
}
