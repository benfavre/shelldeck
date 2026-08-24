// ShellDeck patch: SDPATCH-028 — shared message composer.
//
// Five ShellDeck surfaces ask the user to write something and send it: the AI
// assistant in its dock window and in its workspace sheet, the "new request"
// sheet, the request comment field, and the support reply field. Before this
// component each of them assembled the same idea differently — the field was an
// `Input` here and a bordered `div` there, the send control sat beside the field
// in one place and below it in another, and the AI entry point floated above the
// field in a third. `.agents/ui-components.md` calls that a fork, not a
// preference, and asks for one shape reused rather than four re-invented.
//
// The invariant is the container: one rounded frame that owns the border, the
// focus ring and the padding, holding an `InputVariant::Bare` field so no second
// frame is drawn inside it. What varies between surfaces is only the *contents*
// of the two slot rows — a context row above the field, a footer row below it —
// and whether the commit control is a round arrow (sending a message) or a named
// button (committing a form).

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, AnyElement, App, Div, Entity, FocusHandle, IntoElement, RenderOnce, SharedString,
    Styled, Window,
};

use crate::components::input::{Input, InputVariant};
use crate::components::input_state::InputState;
use crate::layout::HStack;
use crate::theme::use_theme;

/// How the composer commits what was typed.
#[derive(Clone)]
pub enum ComposerCommit {
    /// Round, icon-only arrow. Sending a message — the assistant, a comment, a
    /// support reply.
    Send,
    /// Named button. Committing a form is not the same act as sending a
    /// message, so "Créer" keeps its word.
    Labeled(SharedString),
}

#[derive(IntoElement)]
pub struct Composer {
    id: SharedString,
    /// The built-in field. `None` when the caller supplies its own — the
    /// Support surfaces write with an `Editor`, not an `Input`, and what this
    /// component contributes is the frame and the slots, not the field type.
    state: Option<Entity<InputState>>,
    custom_field: Option<AnyElement>,
    custom_focus: Option<FocusHandle>,
    placeholder: SharedString,
    min_rows: usize,
    max_rows: Option<usize>,
    disabled: bool,
    /// Chips above the field: what this message is about.
    context: Vec<AnyElement>,
    /// Rendered between the context row and the field — the request sheet puts
    /// its title line and separator here.
    lead: Option<AnyElement>,
    /// Footer, left of the spacer: attach, target, AI.
    actions: Vec<AnyElement>,
    /// Footer, right of the spacer and before the commit control: the model
    /// picker, the reply visibility, …
    options: Vec<AnyElement>,
    commit: Option<ComposerCommit>,
    commit_enabled: bool,
    /// One handler drives both the commit control and the Enter key. Wiring
    /// them separately is how a composer ends up sending on click but not on
    /// Enter (or the reverse) — the component makes that impossible.
    on_commit: Option<Rc<dyn Fn(&mut App) + 'static>>,
    /// Rendered below the frame, outside it: keyboard hints, attachment
    /// counters, the execution mode. Never inside — the frame is the field.
    footnote: Option<AnyElement>,
}

impl Composer {
    pub fn new(id: impl Into<SharedString>, state: &Entity<InputState>) -> Self {
        Self::build(id.into(), Some(state.clone()), None, None)
    }

    /// Same frame and slots, but the caller renders the field. `focus_handle`
    /// is what makes the frame light up, since the component can no longer ask
    /// the field itself.
    pub fn with_field(
        id: impl Into<SharedString>,
        focus_handle: FocusHandle,
        field: impl IntoElement,
    ) -> Self {
        Self::build(
            id.into(),
            None,
            Some(field.into_any_element()),
            Some(focus_handle),
        )
    }

    fn build(
        id: SharedString,
        state: Option<Entity<InputState>>,
        custom_field: Option<AnyElement>,
        custom_focus: Option<FocusHandle>,
    ) -> Self {
        Self {
            id,
            state,
            custom_field,
            custom_focus,
            placeholder: SharedString::default(),
            min_rows: 2,
            max_rows: Some(10),
            disabled: false,
            context: Vec::new(),
            lead: None,
            actions: Vec::new(),
            options: Vec::new(),
            commit: Some(ComposerCommit::Send),
            commit_enabled: true,
            on_commit: None,
            footnote: None,
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn min_rows(mut self, rows: usize) -> Self {
        self.min_rows = rows.max(1);
        self
    }

    pub fn max_rows(mut self, rows: impl Into<Option<usize>>) -> Self {
        self.max_rows = rows.into();
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn context(mut self, chip: impl IntoElement) -> Self {
        self.context.push(chip.into_any_element());
        self
    }

    pub fn lead(mut self, lead: impl IntoElement) -> Self {
        self.lead = Some(lead.into_any_element());
        self
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.actions.push(action.into_any_element());
        self
    }

    pub fn option(mut self, option: impl IntoElement) -> Self {
        self.options.push(option.into_any_element());
        self
    }

    pub fn commit(mut self, commit: ComposerCommit) -> Self {
        self.commit = Some(commit);
        self
    }

    /// ShellDeck patch: SDPATCH-028 — callers that supply a richer commit
    /// control through an option still need the shared frame and Enter wiring,
    /// but must not render the default send control beside their own button.
    pub fn without_commit(mut self) -> Self {
        self.commit = None;
        self
    }

    /// Greys the commit control without removing it, so the row keeps its shape
    /// while there is nothing to send.
    pub fn commit_enabled(mut self, enabled: bool) -> Self {
        self.commit_enabled = enabled;
        self
    }

    /// Drives the commit control *and* the Enter key, so the two can never
    /// drift apart.
    pub fn on_commit(mut self, handler: impl Fn(&mut App) + 'static) -> Self {
        self.on_commit = Some(Rc::new(handler));
        self
    }

    pub fn footnote(mut self, footnote: impl IntoElement) -> Self {
        self.footnote = Some(footnote.into_any_element());
        self
    }

    fn focus_handle(&self, cx: &App) -> Option<FocusHandle> {
        self.custom_focus
            .clone()
            .or_else(|| self.state.as_ref().map(|state| state.read(cx).focus_handle(cx)))
    }
}

impl RenderOnce for Composer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = use_theme();
        let focused = self
            .focus_handle(cx)
            .is_some_and(|handle| handle.is_focused(window))
            && !self.disabled;

        let commit_ready = self.commit_enabled && !self.disabled;
        let field: AnyElement = match (self.custom_field, self.state.as_ref()) {
            (Some(custom), _) => custom,
            (None, Some(state)) => {
                let field = Input::new(state)
                    .variant(InputVariant::Bare)
                    .multi_line(true)
                    .min_rows(self.min_rows)
                    .placeholder(self.placeholder.clone())
                    .disabled(self.disabled);
                let field = match self.max_rows {
                    Some(rows) => field.max_rows(rows),
                    None => field,
                };
                // Enter commits through the very same closure as the button.
                match (commit_ready, self.on_commit.clone()) {
                    (true, Some(handler)) => {
                        field.on_enter(move |_value, cx| handler(cx)).into_any_element()
                    }
                    _ => field.into_any_element(),
                }
            }
            (None, None) => div().into_any_element(),
        };

        let has_footer =
            !self.actions.is_empty() || !self.options.is_empty() || self.commit.is_some();

        // Two composers can share a window (support list + detail, a sheet over
        // the assistant), so the commit control's element id is derived from the
        // composer's rather than being a constant.
        let commit_id = SharedString::from(format!("{}-commit", self.id));

        let mut frame: Div = div()
            .flex()
            .flex_col()
            .w_full()
            .min_w(px(0.0))
            .bg(theme.tokens.background)
            .border_1()
            .rounded(px(13.0))
            .overflow_hidden();
        // The frame — not the field — carries the focus state, which is the
        // whole point of `InputVariant::Bare`.
        if focused {
            frame = frame.border_color(theme.tokens.ring);
        } else {
            frame = frame.border_color(theme.tokens.border);
        }

        if !self.context.is_empty() {
            frame = frame.child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(5.0))
                    .px(px(10.0))
                    .pt(px(9.0))
                    .children(self.context),
            );
        }
        if let Some(lead) = self.lead {
            frame = frame.child(lead);
        }
        frame = frame.child(div().w_full().min_w(px(0.0)).px(px(2.0)).child(field));

        if has_footer {
            let mut footer = HStack::new()
                .items_center()
                .gap(px(4.0))
                .w_full()
                .min_w(px(0.0))
                .pl(px(8.0))
                .pr(px(7.0))
                .pb(px(7.0))
                .children(self.actions)
                .child(div().flex_1().min_w(px(0.0)))
                .children(self.options);

            if let Some(commit) = self.commit {
                let enabled = commit_ready;
                let on_commit = self.on_commit.clone();
                // `Icon` falls back to `theme.tokens.primary` when given no
                // colour (see `icon.rs`) — it does *not* inherit the parent's
                // `text_color`. On a primary-filled button that paints the
                // arrow primary-on-primary, i.e. invisible. The glyph colour
                // must therefore be stated explicitly on both branches.
                let glyph = if enabled {
                    theme.tokens.primary_foreground
                } else {
                    theme.tokens.muted_foreground
                };
                let commit_el = match commit {
                    ComposerCommit::Send => {
                        let mut button = div()
                            .id(commit_id.clone())
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .size(px(28.0))
                            .ml(px(4.0))
                            .rounded_full();
                        if enabled {
                            button = button.bg(theme.tokens.primary).cursor_pointer();
                        } else {
                            button = button.bg(theme.tokens.muted);
                        }
                        button.child(
                            crate::components::icon::Icon::new("arrow-up")
                                .size(px(15.0))
                                .color(glyph),
                        )
                    }
                    ComposerCommit::Labeled(label) => {
                        let mut button = div()
                            .id(commit_id.clone())
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .h(px(28.0))
                            .px(px(13.0))
                            .ml(px(4.0))
                            .rounded(px(7.0))
                            .text_size(px(12.0))
                            .text_color(glyph)
                            .child(label);
                        if enabled {
                            button = button.bg(theme.tokens.primary).cursor_pointer();
                        } else {
                            button = button.bg(theme.tokens.muted);
                        }
                        button
                    }
                };
                let commit_el = match (enabled, on_commit) {
                    (true, Some(handler)) => commit_el.on_click(move |_event, _window, cx| {
                        handler(cx);
                    }),
                    _ => commit_el,
                };
                footer = footer.child(commit_el);
            }
            frame = frame.child(footer);
        }

        let mut root = div().flex().flex_col().w_full().min_w(px(0.0)).child(frame);
        if let Some(footnote) = self.footnote {
            root = root.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .flex_wrap()
                    .px(px(6.0))
                    .pt(px(7.0))
                    .text_size(px(10.5))
                    .text_color(theme.tokens.muted_foreground)
                    .child(footnote),
            );
        }
        root
    }
}
