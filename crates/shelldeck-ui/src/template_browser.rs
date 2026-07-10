use crate::scale::px;
use adabraka_ui::components::input::{Input, InputSize, InputState};
use gpui::prelude::*;
use gpui::*;

use shelldeck_core::models::script::{Script, ScriptCategory};
use shelldeck_core::models::templates::{all_templates, ScriptTemplate};

use crate::syntax::highlight::render_code_block_with_language;
use crate::t;
use crate::theme::ShellDeckColors;

#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum TemplateBrowserEvent {
    Import(Script),
    Cancel,
}

impl EventEmitter<TemplateBrowserEvent> for TemplateBrowser {}

pub struct TemplateBrowser {
    templates: Vec<ScriptTemplate>,
    /// Real adabraka `Input` state for the templates search bar.
    search_state: Entity<InputState>,
    /// Sync'd copy of `search_state.content` for `filtered_templates`.
    search_query: String,
    selected_category: Option<ScriptCategory>,
    selected_index: usize,
    focus_handle: FocusHandle,
}

impl TemplateBrowser {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            templates: all_templates(),
            search_state: cx.new(InputState::new),
            search_query: String::new(),
            selected_category: None,
            selected_index: 0,
            focus_handle: cx.focus_handle(),
        }
    }

    pub fn focus(&self, window: &mut Window) {
        self.focus_handle.focus(window);
    }

    fn filtered_templates(&self) -> Vec<&ScriptTemplate> {
        let query_lower = self.search_query.to_lowercase();
        self.templates
            .iter()
            .filter(|t| {
                if let Some(cat) = self.selected_category {
                    if t.category != cat {
                        return false;
                    }
                }
                if !query_lower.is_empty() {
                    let name_match = t.name.to_lowercase().contains(&query_lower);
                    let desc_match = t.description.to_lowercase().contains(&query_lower);
                    if !name_match && !desc_match {
                        return false;
                    }
                }
                true
            })
            .collect()
    }

    /// Text keys are handled inside the focused `Input`; here we only catch
    /// Escape (cancel) and Up/Down navigation. Enter is wired to the input's
    /// `on_enter` callback so it can fire the import event.
    fn handle_key_down(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "escape" => {
                cx.emit(TemplateBrowserEvent::Cancel);
            }
            "up" => {
                if self.selected_index > 0 {
                    self.selected_index -= 1;
                }
                cx.notify();
            }
            "down" => {
                let count = self.filtered_templates().len();
                if count > 0 && self.selected_index < count - 1 {
                    self.selected_index += 1;
                }
                cx.notify();
            }
            _ => {}
        }
    }

    fn import_selected(&self, cx: &mut Context<Self>) {
        let filtered = self.filtered_templates();
        if let Some(tmpl) = filtered.get(self.selected_index) {
            cx.emit(TemplateBrowserEvent::Import(tmpl.to_script()));
        }
    }
}

impl Render for TemplateBrowser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.focus_handle.focus(window);

        let filtered = self.filtered_templates();
        let selected_tmpl = filtered.get(self.selected_index).cloned();

        // Category tabs
        let mut tabs = div().flex().flex_wrap().gap(px(4.0));

        // "All" tab
        let all_selected = self.selected_category.is_none();
        let mut all_tab = div()
            .id("tmpl-cat-all")
            .px(px(8.0))
            .py(px(3.0))
            .rounded(px(4.0))
            .text_size(px(11.0))
            .font_weight(FontWeight::MEDIUM)
            .cursor_pointer()
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.selected_category = None;
                this.selected_index = 0;
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
        tabs = tabs.child(all_tab.child(t!("template_browser.all").to_string()));

        for cat in ScriptCategory::ALL {
            if *cat == ScriptCategory::Uncategorized || *cat == ScriptCategory::Custom {
                continue;
            }
            let selected = self.selected_category == Some(*cat);
            let cat_val = *cat;
            let mut tab = div()
                .id(ElementId::from(SharedString::from(format!(
                    "tmpl-cat-{}",
                    cat.label()
                ))))
                .px(px(8.0))
                .py(px(3.0))
                .rounded(px(4.0))
                .text_size(px(11.0))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.selected_category = Some(cat_val);
                    this.selected_index = 0;
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
            tabs = tabs.child(tab.child(cat.label().to_string()));
        }

        // Real `Input` search bar. Enter imports the selected template.
        let search_bar = Input::new(&self.search_state)
            .size(InputSize::Sm)
            .placeholder(t!("template_browser.search_placeholder").to_string())
            .clearable(true)
            .prefix(
                svg()
                    .path("icons/lucide/search.svg")
                    .size(px(12.0))
                    .flex_shrink_0()
                    .text_color(ShellDeckColors::text_muted()),
            )
            .on_change({
                let entity = cx.entity();
                move |value, cx| {
                    entity.update(cx, |this, cx| {
                        this.search_query = value.to_string();
                        this.selected_index = 0;
                        cx.notify();
                    });
                }
            })
            .on_enter({
                let entity = cx.entity();
                move |_v, cx| {
                    entity.update(cx, |this, cx| this.import_selected(cx));
                }
            });

        // Template list
        let mut template_list = div()
            .flex()
            .flex_col()
            .w(px(300.0))
            .h_full()
            .border_r_1()
            .border_color(ShellDeckColors::border())
            .id("template-list")
            .overflow_y_scroll();

        if filtered.is_empty() {
            template_list = template_list.child(
                div()
                    .px(px(12.0))
                    .py(px(24.0))
                    .text_size(px(12.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("template_browser.no_matches").to_string()),
            );
        }

        for (i, tmpl) in filtered.iter().enumerate() {
            let is_selected = i == self.selected_index;
            let (r, g, b) = tmpl.language.badge_color();
            let badge_color = gpui::hsla(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0);
            let _tmpl_id = tmpl.id.clone();

            let mut item = div()
                .id(ElementId::from(SharedString::from(format!("tmpl-{}", i))))
                .flex()
                .flex_col()
                .gap(px(2.0))
                .px(px(12.0))
                .py(px(8.0))
                .cursor_pointer()
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.selected_index = i;
                    cx.notify();
                }));

            if is_selected {
                item = item
                    .bg(ShellDeckColors::primary().opacity(0.12))
                    .border_l_2()
                    .border_color(ShellDeckColors::primary());
            } else {
                item = item.hover(|el| el.bg(ShellDeckColors::hover_bg()));
            }

            item = item
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.0))
                        // Language badge
                        .child(
                            div()
                                .text_size(px(9.0))
                                .px(px(4.0))
                                .py(px(1.0))
                                .rounded(px(3.0))
                                .bg(badge_color.opacity(0.15))
                                .text_color(badge_color)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(tmpl.language.badge().to_string()),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(ShellDeckColors::text_primary())
                                .font_weight(FontWeight::MEDIUM)
                                .child(tmpl.name.clone()),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(ShellDeckColors::text_muted())
                        .child(tmpl.description.clone()),
                );

            template_list = template_list.child(item);
        }

        // Preview panel
        let mut preview = div()
            .flex()
            .flex_col()
            .flex_grow()
            .h_full()
            .min_w_0()
            .overflow_hidden();

        if let Some(tmpl) = selected_tmpl {
            let tmpl_id_for_btn = tmpl.id.clone();
            let tmpl_name = tmpl.name.clone();
            let tmpl_desc = tmpl.description.clone();
            let tmpl_body = tmpl.body.clone();
            let tmpl_lang = tmpl.language.clone();
            let tmpl_cat_label = tmpl.category.label().to_string();

            // Preview header
            preview = preview.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .px(px(16.0))
                    .py(px(12.0))
                    .border_b_1()
                    .border_color(ShellDeckColors::border())
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.0))
                            .child(
                                div()
                                    .text_size(px(15.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(ShellDeckColors::text_primary())
                                    .child(tmpl_name),
                            )
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(ShellDeckColors::text_muted())
                                    .child(tmpl_desc),
                            )
                            .child(
                                div().flex().items_center().gap(px(6.0)).child(
                                    div()
                                        .text_size(px(10.0))
                                        .px(px(4.0))
                                        .py(px(1.0))
                                        .rounded(px(3.0))
                                        .bg(ShellDeckColors::badge_bg())
                                        .text_color(ShellDeckColors::text_muted())
                                        .child(tmpl_cat_label),
                                ),
                            ),
                    )
                    .child(
                        div()
                            .id("import-template-btn")
                            .cursor_pointer()
                            .px(px(12.0))
                            .py(px(6.0))
                            .rounded(px(6.0))
                            .bg(ShellDeckColors::primary())
                            .text_color(ShellDeckColors::bg_primary())
                            .text_size(px(12.0))
                            .font_weight(FontWeight::MEDIUM)
                            .hover(|el| el.opacity(0.9))
                            .on_click(cx.listener(move |_this, _: &ClickEvent, _, cx| {
                                let templates = all_templates();
                                if let Some(t) = templates.iter().find(|t| t.id == tmpl_id_for_btn)
                                {
                                    cx.emit(TemplateBrowserEvent::Import(t.to_script()));
                                }
                            }))
                            .child(t!("template_browser.add_to_scripts").to_string()),
                    ),
            );

            // Code preview
            preview = preview.child(
                div()
                    .flex_grow()
                    .min_h_0()
                    .p(px(16.0))
                    .id("template-preview")
                    .overflow_y_scroll()
                    .child(
                        div()
                            .w_full()
                            .rounded(px(8.0))
                            .bg(ShellDeckColors::terminal_bg())
                            .border_1()
                            .border_color(ShellDeckColors::border())
                            .font_family("JetBrains Mono")
                            .overflow_hidden()
                            .child(render_code_block_with_language(
                                &tmpl_body, None, false, &tmpl_lang,
                            )),
                    ),
            );
        } else {
            preview = preview.child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .flex_grow()
                    .text_size(px(13.0))
                    .text_color(ShellDeckColors::text_muted())
                    .child(t!("template_browser.select_to_preview").to_string()),
            );
        }

        // Main overlay
        div()
            .id("template-browser-overlay")
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
                    .w(px(800.0))
                    .h(px(560.0))
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
                                    .child(t!("template_browser.title").to_string()),
                            )
                            .child(
                                div()
                                    .id("close-template-browser")
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .cursor_pointer()
                                    .text_color(ShellDeckColors::text_muted())
                                    .hover(|el| el.text_color(ShellDeckColors::text_primary()))
                                    .on_click(cx.listener(|_this, _: &ClickEvent, _, cx| {
                                        cx.emit(TemplateBrowserEvent::Cancel);
                                    }))
                                    .child(
                                        svg()
                                            .path("icons/lucide/x.svg")
                                            .size(px(14.0))
                                            .text_color(ShellDeckColors::text_muted()),
                                    ),
                            ),
                    )
                    // Category tabs + search
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.0))
                            .px(px(16.0))
                            .py(px(8.0))
                            .border_b_1()
                            .border_color(ShellDeckColors::border())
                            .child(tabs)
                            .child(search_bar),
                    )
                    // Content: list + preview
                    .child(
                        div()
                            .flex()
                            .flex_grow()
                            .min_h_0()
                            .child(template_list)
                            .child(preview),
                    ),
            )
    }
}
