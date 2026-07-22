//! Select component - Dropdown select with keyboard navigation.

use crate::components::icon::Icon;
use crate::components::icon_source::IconSource;
use crate::components::input::{Input, InputSize, InputVariant};
use crate::components::input_state::InputState;
use crate::components::scrollable::scrollable_vertical;
use crate::theme::use_theme;
use gpui::{prelude::*, *};

actions!(select, [SelectUp, SelectDown, SelectConfirm, SelectCancel]);

const DROPDOWN_MARGIN: Pixels = px(4.0);
// ShellDeck patch: SDPATCH-022 — large searchable menus must not instantiate
// every option. Above this threshold, the dropdown uses `uniform_list` and
// renders only the visible rows.
const VIRTUAL_LIST_THRESHOLD: usize = 50;

#[derive(Clone, Debug)]
pub enum SelectEvent {
    Change,
}

#[derive(Clone)]
pub struct SelectOption<T: Clone> {
    pub value: T,
    pub label: SharedString,
    pub group: Option<SharedString>,
    pub icon: Option<IconSource>,
}

impl<T: Clone> SelectOption<T> {
    pub fn new(value: T, label: impl Into<SharedString>) -> Self {
        Self {
            value,
            label: label.into(),
            group: None,
            icon: None,
        }
    }

    pub fn with_group(mut self, group: impl Into<SharedString>) -> Self {
        self.group = Some(group.into());
        self
    }

    pub fn with_icon(mut self, icon: impl Into<IconSource>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}

pub struct Select<T: Clone + 'static> {
    focus_handle: FocusHandle,
    options: Vec<SelectOption<T>>,
    selected_index: Option<usize>,
    highlighted_index: Option<usize>,
    placeholder: Option<SharedString>,
    open: bool,
    disabled: bool,
    searchable: bool,
    clearable: bool,
    loading: bool,
    search_query: String,
    // ShellDeck patch: SDPATCH-022 — use the library's real text input so
    // searchable Selects inherit native caret, selection, and IME behavior.
    search_input: Entity<InputState>,
    // ShellDeck patch: SDPATCH-022 — callers can localize the searchable
    // dropdown's input placeholder instead of inheriting an English literal.
    search_placeholder: SharedString,
    on_change: Option<Box<dyn Fn(&T, &mut Window, &mut App) + Send + Sync + 'static>>,
    bounds: Bounds<Pixels>,
    // ShellDeck patch: SDPATCH-022 — deferred popup contents are outside the
    // trigger's hit-test tree, so retain their real bounds for outside-clicks.
    dropdown_bounds: Option<Bounds<Pixels>>,
    leading_icon: Option<IconSource>,
    style: StyleRefinement,
}

impl<T: Clone + 'static> Select<T> {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let search_input = cx.new(InputState::new);
        Self {
            focus_handle: cx.focus_handle(),
            options: Vec::new(),
            selected_index: None,
            highlighted_index: None,
            placeholder: None,
            open: false,
            disabled: false,
            searchable: false,
            clearable: false,
            loading: false,
            search_query: String::new(),
            search_input,
            // ShellDeck patch: SDPATCH-022 — preserve the upstream-facing
            // English default while allowing localized callers to override it.
            search_placeholder: "Type to search...".into(),
            on_change: None,
            bounds: Bounds::default(),
            dropdown_bounds: None,
            leading_icon: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn options(mut self, options: Vec<SelectOption<T>>) -> Self {
        self.options = options;
        self
    }

    pub fn selected_index(mut self, index: Option<usize>) -> Self {
        self.selected_index = index;
        self.highlighted_index = index;
        self
    }

    pub fn placeholder<S: Into<SharedString>>(mut self, placeholder: S) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn searchable(mut self, searchable: bool) -> Self {
        self.searchable = searchable;
        self
    }

    /// ShellDeck patch: SDPATCH-022 — localize the input shown inside an open
    /// searchable dropdown.
    pub fn search_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.search_placeholder = placeholder.into();
        self
    }

    pub fn clearable(mut self, clearable: bool) -> Self {
        self.clearable = clearable;
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn on_change<F: Fn(&T, &mut Window, &mut App) + Send + Sync + 'static>(
        mut self,
        f: F,
    ) -> Self {
        self.on_change = Some(Box::new(f));
        self
    }

    pub fn leading_icon(mut self, icon: impl Into<IconSource>) -> Self {
        self.leading_icon = Some(icon.into());
        self
    }

    pub fn selected_value(&self) -> Option<&T> {
        self.selected_index
            .and_then(|i| self.options.get(i))
            .map(|opt| &opt.value)
    }

    pub fn selected_label(&self) -> Option<&SharedString> {
        self.selected_index
            .and_then(|i| self.options.get(i))
            .map(|opt| &opt.label)
    }

    fn filtered_options(&self) -> Vec<(usize, &SelectOption<T>)> {
        if self.search_query.is_empty() {
            self.options.iter().enumerate().collect()
        } else {
            let query_lower = self.search_query.to_lowercase();
            self.options
                .iter()
                .enumerate()
                .filter(|(_, opt)| opt.label.to_lowercase().contains(&query_lower))
                .collect()
        }
    }

    // ShellDeck patch: SDPATCH-022 — GPUI clips flex-row text before its
    // CSS-style ellipsis is painted. Keep the full label for filtering, but
    // render a deterministic Unicode ellipsis inside constrained menus.
    fn ellipsized_option_label(label: &str) -> SharedString {
        const MAX_VISIBLE_CHARS: usize = 44;
        let mut chars = label.chars();
        let prefix: String = chars
            .by_ref()
            .take(MAX_VISIBLE_CHARS.saturating_sub(1))
            .collect();
        if chars.next().is_some() {
            format!("{prefix}…").into()
        } else {
            label.to_string().into()
        }
    }

    fn toggle_dropdown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.disabled {
            self.open = !self.open;
            if self.open {
                if self.searchable {
                    window.focus(&self.search_input.read(cx).focus_handle(cx));
                } else {
                    window.focus(&self.focus_handle);
                }
                self.highlighted_index = self.selected_index.or(Some(0));
            }
            cx.notify();
        }
    }

    fn close_dropdown(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        self.dropdown_bounds = None;
        self.highlighted_index = self.selected_index;
        self.search_query.clear();
        self.search_input.update(cx, InputState::reset);
        cx.notify();
    }

    fn clear_selection(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.selected_index = None;
        self.highlighted_index = None;

        cx.emit(SelectEvent::Change);
        cx.notify();
    }

    fn select_option(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.options.len() {
            self.selected_index = Some(index);
            self.highlighted_index = Some(index);
            self.open = false;

            cx.emit(SelectEvent::Change);
            cx.notify();

            if let Some(ref cb) = self.on_change {
                if let Some(option) = self.options.get(index) {
                    (cb)(&option.value, window, cx);
                }
            }
        }
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }

        let filtered = self.filtered_options();
        if filtered.is_empty() {
            return;
        }

        let current_pos = self
            .highlighted_index
            .and_then(|idx| filtered.iter().position(|(orig_idx, _)| *orig_idx == idx));

        let new_pos = match current_pos {
            Some(0) => filtered.len() - 1,
            Some(pos) => pos - 1,
            None => filtered.len() - 1,
        };

        self.highlighted_index = Some(filtered[new_pos].0);
        cx.notify();
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }

        let filtered = self.filtered_options();
        if filtered.is_empty() {
            return;
        }

        let current_pos = self
            .highlighted_index
            .and_then(|idx| filtered.iter().position(|(orig_idx, _)| *orig_idx == idx));

        let new_pos = match current_pos {
            Some(pos) if pos < filtered.len() - 1 => pos + 1,
            Some(_) => 0,
            None => 0,
        };

        self.highlighted_index = Some(filtered[new_pos].0);
        cx.notify();
    }

    fn select_confirm(&mut self, _: &SelectConfirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            if let Some(idx) = self.highlighted_index {
                self.select_option(idx, window, cx);
            }
        } else {
            self.toggle_dropdown(window, cx);
        }
    }

    fn select_cancel(&mut self, _: &SelectCancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.close_dropdown(cx);
        }
    }
}

impl<T: Clone + 'static> Styled for Select<T> {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl<T: Clone + 'static> Render for Select<T> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = use_theme();
        let user_style = self.style.clone();

        let display_text = self
            .selected_label()
            .cloned()
            .or_else(|| self.placeholder.clone())
            .unwrap_or_else(|| "Select...".into());

        let open = self.open;
        let highlighted_idx = self.highlighted_index;
        let bounds = self.bounds;

        let maybe_selected_icon: Option<IconSource> = self
            .selected_index
            .and_then(|i| self.options.get(i))
            .and_then(|opt| opt.icon.clone());
        let leading_icon = self.leading_icon.clone().or(maybe_selected_icon);

        let trigger = div()
            .id("select-trigger")
            .relative()
            .flex()
            .items_center()
            .justify_between()
            .h(px(40.0))
            .px(px(12.0))
            .bg(theme.tokens.background)
            .border_1()
            .border_color(if open {
                theme.tokens.ring
            } else {
                theme.tokens.input
            })
            .rounded(theme.tokens.radius_md)
            .text_color(if self.selected_index.is_some() {
                theme.tokens.foreground
            } else {
                theme.tokens.muted_foreground
            })
            .text_size(px(14.0))
            .font_family(theme.tokens.font_family.clone())
            .cursor(if self.disabled {
                CursorStyle::Arrow
            } else {
                CursorStyle::PointingHand
            })
            .when(!self.disabled, |div: Stateful<Div>| {
                div.hover(|mut style| {
                    style.border_color = Some(theme.tokens.ring);
                    style
                })
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    this.toggle_dropdown(window, cx);
                }),
            )
            .child(
                div()
                    // ShellDeck patch: truncate long labels — GPUI flex children ignore
                    // parent width caps without min_w(0) + overflow_hidden.
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .flex_grow()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .when_some(leading_icon.as_ref(), |div, src| {
                        div.child(
                            Icon::new(src.clone())
                                .size(px(14.0))
                                .color(theme.tokens.muted_foreground),
                        )
                    })
                    .child(
                        div()
                            .flex_grow()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .truncate()
                            .child(display_text),
                    ),
            )
            .child(
                div()
                    .ml(px(8.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        if self.clearable && self.selected_index.is_some() && !self.disabled {
                            div()
                                .w(px(20.0))
                                .h(px(20.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(10.0))
                                .cursor(CursorStyle::PointingHand)
                                .hover(|mut style| {
                                    style.background = Some(theme.tokens.muted.into());
                                    style
                                })
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(|this, _event: &MouseDownEvent, window, cx| {
                                        this.clear_selection(window, cx);
                                    }),
                                )
                                .child(
                                    Icon::new("x")
                                        .size(px(14.0))
                                        .color(theme.tokens.muted_foreground),
                                )
                                .into_any_element()
                        } else {
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(if open { "arrow-up" } else { "arrow-down" })
                                        .size(px(14.0))
                                        .color(theme.tokens.muted_foreground),
                                )
                                .into_any_element()
                        },
                    ),
            )
            .child({
                let entity = cx.entity().clone();
                canvas(
                    move |bounds, _, cx| {
                        entity.update(cx, |this, _| {
                            this.bounds = bounds;
                        })
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full()
            });

        let searchable = self.searchable;
        // ShellDeck patch: SDPATCH-022 — render the caller-provided localized
        // search hint when the query is empty.
        let search_placeholder = self.search_placeholder.clone();
        let search_input = self.search_input.clone();
        let filtered_count = self.filtered_options().len();
        let loading = self.loading;

        div()
            .relative()
            .w_full()
            .min_w(px(0.0))
            .overflow_hidden()
            .key_context("Select")
            .track_focus(&self.focus_handle)
            .when(open && !self.disabled, |this: Div| {
                this.on_action(cx.listener(Select::select_up))
                    .on_action(cx.listener(Select::select_down))
                    .on_action(cx.listener(Select::select_confirm))
                    .on_action(cx.listener(Select::select_cancel))
            })
            .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _window, cx| {
                // ShellDeck patch: SDPATCH-022 — `deferred` popup children
                // trigger `on_mouse_down_out` on the Select root. A click
                // inside the measured popup is not an outside click.
                let inside_popup = this
                    .dropdown_bounds
                    .is_some_and(|bounds| bounds.contains(&event.position));
                if !inside_popup && this.open {
                    this.close_dropdown(cx);
                }
            }))
            .child(trigger)
            .when(open, |this| {
                this.child(
                    deferred(
                        anchored()
                            .snap_to_window_with_margin(Edges::all(DROPDOWN_MARGIN))
                            .child(
                                div()
                                    .occlude()
                                    .relative()
                                    .w(bounds.size.width)
                                    .child({
                                        let entity = cx.entity().clone();
                                        canvas(
                                            move |bounds, _, cx| {
                                                entity.update(cx, |this, _| {
                                                    this.dropdown_bounds = Some(bounds);
                                                })
                                            },
                                            |_, _, _, _| {},
                                        )
                                        .absolute()
                                        .size_full()
                                    })
                                    .child(
                                        div()
                                            .occlude()
                                            .mt(DROPDOWN_MARGIN)
                                            .bg(theme.tokens.popover)
                                            .border_2()
                                            .border_color(theme.tokens.ring)
                                            .rounded(theme.tokens.radius_md)
                                            .shadow_xl()
                                            .overflow_hidden()
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .when(searchable, |this| {
                                                        this.child(
                                                            div()
                                                                .px(px(12.0))
                                                                .pt(px(8.0))
                                                                .pb(px(4.0))
                                                                .border_b_1()
                                                                .border_color(theme.tokens.border)
                                                                // ShellDeck patch: SDPATCH-022 — render the real Input;
                                                                // its existing caret/selection fixes now apply here too.
                                                                .child(Input::new(&search_input)
                                                                    .size(InputSize::Sm)
                                                                    .variant(InputVariant::Ghost)
                                                                    .placeholder(search_placeholder.clone())
                                                                    .on_change({
                                                                        let entity = cx.entity().clone();
                                                                        move |value, cx| {
                                                                            entity.update(cx, |this, cx| {
                                                                                this.search_query = value.to_string();
                                                                                this.highlighted_index = this
                                                                                    .filtered_options()
                                                                                    .first()
                                                                                    .map(|(index, _)| *index);
                                                                                cx.notify();
                                                                            });
                                                                        }
                                                                    }))
                                                        )
                                                    })
                                                    .child(
                                                        if !loading && filtered_count >= VIRTUAL_LIST_THRESHOLD {
                                                            // ShellDeck patch: SDPATCH-022 — a fixed-height row
                                                            // lets GPUI virtualize large filtered option sets.
                                                            // The group is rendered inside the row instead of as
                                                            // a variable-height heading.
                                                            uniform_list(
                                                                "select-options-virtual",
                                                                filtered_count,
                                                                cx.processor(|this, range: std::ops::Range<usize>, _window, cx| {
                                                                    let theme = use_theme();
                                                                    let filtered_indices: Vec<usize> = this
                                                                        .filtered_options()
                                                                        .into_iter()
                                                                        .map(|(index, _)| index)
                                                                        .collect();
                                                                    let mut rows = Vec::with_capacity(range.len());

                                                                    for display_index in range {
                                                                        let Some(&index) = filtered_indices.get(display_index) else {
                                                                            continue;
                                                                        };
                                                                        let option = &this.options[index];
                                                                        let is_selected = this.selected_index == Some(index);
                                                                        let is_highlighted = this.highlighted_index == Some(index);
                                                                        let item_fg = if is_highlighted {
                                                                            theme.tokens.accent_foreground
                                                                        } else if is_selected {
                                                                            theme.tokens.primary
                                                                        } else {
                                                                            theme.tokens.popover_foreground
                                                                        };

                                                                        rows.push(
                                                                            div()
                                                                                .id(("select-option-virtual", index))
                                                                                .w_full()
                                                                                .h(px(48.0))
                                                                                .px(px(12.0))
                                                                                .flex()
                                                                                .items_center()
                                                                                .text_color(item_fg)
                                                                                .bg(if is_highlighted {
                                                                                    theme.tokens.accent
                                                                                } else {
                                                                                    gpui::transparent_black()
                                                                                })
                                                                                .hover(|mut style| {
                                                                                    style.background = Some(theme.tokens.accent.into());
                                                                                    style
                                                                                })
                                                                                .cursor(CursorStyle::PointingHand)
                                                                                .font_family(theme.tokens.font_family.clone())
                                                                                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                                                                                    this.select_option(index, window, cx);
                                                                                }))
                                                                                .child(
                                                                                    div()
                                                                                        .flex()
                                                                                        .w_full()
                                                                                        .items_center()
                                                                                        .gap(px(8.0))
                                                                                        .min_w(px(0.0))
                                                                                        .overflow_hidden()
                                                                                        .when_some(option.icon.as_ref(), |div, src| {
                                                                                            div.child(
                                                                                                Icon::new(src.clone())
                                                                                                    .size(px(14.0))
                                                                                                    .color(if is_highlighted {
                                                                                                        theme.tokens.accent_foreground
                                                                                                    } else if is_selected {
                                                                                                        theme.tokens.primary
                                                                                                    } else {
                                                                                                        theme.tokens.muted_foreground
                                                                                                    }),
                                                                                            )
                                                                                        })
                                                                                        .child(
                                                                                            div()
                                                                                                .flex()
                                                                                                .flex_col()
                                                                                                .flex_grow()
                                                                                                .min_w(px(0.0))
                                                                                                .overflow_hidden()
                                                                                                .child(
                                                                                                    div()
                                                                                                        .text_size(px(14.0))
                                                                                                        .overflow_hidden()
                                                                                                        .whitespace_nowrap()
                                                                                                        .text_ellipsis()
                                                                                                        .child(Self::ellipsized_option_label(&option.label)),
                                                                                                )
                                                                                                .when_some(option.group.as_ref(), |container, group| {
                                                                                                    container.child(
                                                                                                        div()
                                                                                                            .text_size(px(11.0))
                                                                                                            .text_color(theme.tokens.muted_foreground)
                                                                                                            .overflow_hidden()
                                                                                                            .whitespace_nowrap()
                                                                                                            .text_ellipsis()
                                                                                                            .child(group.clone()),
                                                                                                    )
                                                                                                }),
                                                                                        ),
                                                                                ),
                                                                        );
                                                                    }

                                                                    rows
                                                                }),
                                                            )
                                                            .h(px(300.0))
                                                            .w_full()
                                                            .into_any_element()
                                                        } else {
                                                            div()
                                                                .max_h(px(300.0))
                                                                .child(
                                                                scrollable_vertical({
                                                                let filtered = self.filtered_options();
                                                                let loading = self.loading;

                                                                div()
                                                                    .py(px(4.0))
                                                                    .when(loading, |this| {
                                                                    this.child(
                                                                        div()
                                                                            .px(px(12.0))
                                                                            .py(px(16.0))
                                                                            .flex()
                                                                            .items_center()
                                                                            .justify_center()
                                                                            .gap(px(8.0))
                                                                            .text_size(px(13.0))
                                                                            .font_family(theme.tokens.font_family.clone())
                                                                            .text_color(theme.tokens.muted_foreground)
                                                                            .child(
                                                                                div()
                                                                                    .text_size(px(18.0))
                                                                                    .text_color(theme.tokens.primary)
                                                                                    .child("⟳")
                                                                            )
                                                                            .child("Loading options...")
                                                                    )
                                                                })
                                                                .when(!loading && filtered.is_empty(), |this| {
                                                                    this.child(
                                                                        div()
                                                                            .px(px(12.0))
                                                                            .py(px(16.0))
                                                                            .flex()
                                                                            .items_center()
                                                                            .justify_center()
                                                                            .text_size(px(13.0))
                                                                            .font_family(theme.tokens.font_family.clone())
                                                                            .text_color(theme.tokens.muted_foreground)
                                                                            .child("No results found")
                                                                    )
                                                                })
                                                                .when(!loading && !filtered.is_empty(), |this| {
                                                                    let mut current_group: Option<SharedString> = None;
                                                                    this.children(
                                                                        filtered.iter().flat_map(|(index, option)| {
                                                                            let mut elements = Vec::new();

                                                                            if option.group != current_group {
                                                                                current_group = option.group.clone();
                                                                                if let Some(group_name) = &option.group {
                                                                                    elements.push(
                                                                                        div()
                                                                                            .px(px(12.0))
                                                                                            .pt(px(12.0))
                                                                                            .pb(px(4.0))
                                                                                            .text_size(px(11.0))
                                                                                            .font_family(theme.tokens.font_family.clone())
                                                                                            .font_weight(FontWeight::SEMIBOLD)
                                                                                            .text_color(theme.tokens.muted_foreground)
                                                                                            .child(group_name.clone())
                                                                                            .into_any_element()
                                                                                    );
                                                                                }
                                                                            }

                                                                            let is_selected = self.selected_index == Some(*index);
                                                                            let is_highlighted = highlighted_idx == Some(*index);
                                                                            let index = *index;
                                                                            let item_fg = if is_highlighted {
                                                                                theme.tokens.accent_foreground
                                                                            } else if is_selected {
                                                                                theme.tokens.primary
                                                                            } else {
                                                                                theme.tokens.popover_foreground
                                                                            };

                                                                            elements.push(
                                                                                div()
                                                                                    .px(px(12.0))
                                                                                    .py(px(8.0))
                                                                                    .text_color(item_fg)
                                                                                    .bg(if is_highlighted {
                                                                                        theme.tokens.accent
                                                                                    } else {
                                                                                        gpui::transparent_black()
                                                                                    })
                                                                                    .hover(|mut style| {
                                                                                        style.background = Some(theme.tokens.accent.into());
                                                                                        style
                                                                                    })
                                                                                    .cursor(CursorStyle::PointingHand)
                                                                                    .text_size(px(14.0))
                                                                                    .font_family(theme.tokens.font_family.clone())
                                                                                    .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                                                                                        this.select_option(index, window, cx);
                                                                                    }))
                                                                                    .child(
                                                                                        div()
                                                                                            .flex()
                                                                                            .items_center()
                                                                                            .gap(px(8.0))
                                                                                            .min_w(px(0.0))
                                                                                            .overflow_hidden()
                                                                                            .when_some(option.icon.as_ref(), |div, src| {
                                                                                                div.child(
                                                                                                    Icon::new(src.clone())
                                                                                                        .size(px(14.0))
                                                                                                        .color(if is_highlighted {
                                                                                                            theme.tokens.accent_foreground
                                                                                                        } else if is_selected {
                                                                                                            theme.tokens.primary
                                                                                                        } else {
                                                                                                            theme.tokens.muted_foreground
                                                                                                        })
                                                                                                )
                                                                                            })
                                                                                            .child(
                                                                                                div()
                                                                                                    .flex_grow()
                                                                                                    .min_w(px(0.0))
                                                                                                    .overflow_hidden()
                                                                                                    .truncate()
                                                                                                    .child(Self::ellipsized_option_label(&option.label)),
                                                                                            )
                                                                                    )
                                                                                    .into_any_element()
                                                                            );

                                                                            elements
                                                                        })
                                                                    )
                                                                })
                                                            })
                                                                )
                                                                .into_any_element()
                                                        }
                                                    )
                                            ),
                                    ),
                            ),
                    )
                    .with_priority(1),
                )
            })
            .map(|this| {
                let mut div = this;
                div.style().refine(&user_style);
                div
            })
    }
}

/// Initialize select key bindings
pub fn init_select(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("up", SelectUp, Some("Select")),
        KeyBinding::new("down", SelectDown, Some("Select")),
        KeyBinding::new("enter", SelectConfirm, Some("Select")),
        KeyBinding::new("space", SelectConfirm, Some("Select")),
        KeyBinding::new("escape", SelectCancel, Some("Select")),
    ]);
}

impl<T: Clone + 'static> Focusable for Select<T> {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl<T: Clone + 'static> EventEmitter<SelectEvent> for Select<T> {}
