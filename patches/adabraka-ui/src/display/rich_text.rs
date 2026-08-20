use gpui::prelude::FluentBuilder as _;
use gpui::*;
use std::rc::Rc;

use crate::components::checkbox::{Checkbox, CheckboxSize};
use crate::components::code_block::CodeBlock;
use crate::components::separator::Separator;
use crate::components::text::TextVariant;
use crate::theme::use_theme;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableAlignment {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone)]
pub enum RichInline {
    Text(String),
    Bold(Vec<RichInline>),
    Italic(Vec<RichInline>),
    Strikethrough(Vec<RichInline>),
    Code(String),
    Link {
        text: Vec<RichInline>,
        url: String,
    },
    Image {
        alt: String,
        url: String,
    },
    LineBreak,
    Html(String),
    Styled {
        children: Vec<RichInline>,
        color: Option<Hsla>,
        background_color: Option<Hsla>,
        bold: Option<bool>,
        italic: Option<bool>,
        font_size: Option<f32>,
    },
}

#[derive(Debug, Clone)]
pub struct ListItem {
    pub checked: Option<bool>,
    pub content: Vec<RichInline>,
    pub children: Vec<ListItem>,
}

#[derive(Debug, Clone)]
pub enum RichBlock {
    Paragraph(Vec<RichInline>),
    Heading {
        level: u8,
        content: Vec<RichInline>,
    },
    CodeBlock {
        language: Option<String>,
        code: String,
    },
    BlockQuote(Vec<RichBlock>),
    OrderedList {
        start: u64,
        items: Vec<ListItem>,
    },
    UnorderedList {
        items: Vec<ListItem>,
    },
    Table {
        headers: Vec<Vec<RichInline>>,
        alignments: Vec<TableAlignment>,
        rows: Vec<Vec<Vec<RichInline>>>,
    },
    HorizontalRule,
    Image {
        alt: String,
        url: String,
    },
}

#[derive(Clone)]
pub struct LinkInfo {
    pub range: std::ops::Range<usize>,
    pub url: String,
}

struct FlattenResult {
    text: String,
    runs: Vec<TextRun>,
    links: Vec<LinkInfo>,
}

struct InlineFlattener {
    text: String,
    runs: Vec<TextRun>,
    links: Vec<LinkInfo>,
    font_family: SharedString,
    font_mono: SharedString,
    text_color: Hsla,
    link_color: Hsla,
    code_bg: Hsla,
    bold: bool,
    italic: bool,
    strikethrough: bool,
    in_link: Option<String>,
    color_override: Option<Hsla>,
    bg_override: Option<Hsla>,
}

impl InlineFlattener {
    fn new(
        font_family: SharedString,
        font_mono: SharedString,
        text_color: Hsla,
        link_color: Hsla,
        code_bg: Hsla,
    ) -> Self {
        Self {
            text: String::new(),
            runs: Vec::new(),
            links: Vec::new(),
            font_family,
            font_mono,
            text_color,
            link_color,
            code_bg,
            bold: false,
            italic: false,
            strikethrough: false,
            in_link: None,
            color_override: None,
            bg_override: None,
        }
    }

    fn flatten(mut self, inlines: &[RichInline]) -> FlattenResult {
        self.walk(inlines);
        FlattenResult {
            text: self.text,
            runs: self.runs,
            links: self.links,
        }
    }

    fn walk(&mut self, inlines: &[RichInline]) {
        for inline in inlines {
            match inline {
                RichInline::Text(s) => self.push_text(s, false),
                RichInline::Bold(children) => {
                    let prev = self.bold;
                    self.bold = true;
                    self.walk(children);
                    self.bold = prev;
                }
                RichInline::Italic(children) => {
                    let prev = self.italic;
                    self.italic = true;
                    self.walk(children);
                    self.italic = prev;
                }
                RichInline::Strikethrough(children) => {
                    let prev = self.strikethrough;
                    self.strikethrough = true;
                    self.walk(children);
                    self.strikethrough = prev;
                }
                RichInline::Code(code) => self.push_text(code, true),
                RichInline::Link { text, url } => {
                    let prev = self.in_link.take();
                    self.in_link = Some(url.clone());
                    self.walk(text);
                    self.in_link = prev;
                }
                RichInline::Image { alt, .. } => {
                    if !alt.is_empty() {
                        self.push_text(alt, false);
                    }
                }
                RichInline::LineBreak => self.push_text("\n", false),
                RichInline::Html(_) => {}
                RichInline::Styled {
                    children,
                    color,
                    background_color,
                    bold,
                    italic,
                    font_size: _,
                } => {
                    let prev_bold = self.bold;
                    let prev_italic = self.italic;
                    let prev_color = self.color_override;
                    let prev_bg = self.bg_override;
                    if let Some(true) = bold {
                        self.bold = true;
                    }
                    if let Some(true) = italic {
                        self.italic = true;
                    }
                    if color.is_some() {
                        self.color_override = *color;
                    }
                    if background_color.is_some() {
                        self.bg_override = *background_color;
                    }
                    self.walk(children);
                    self.bold = prev_bold;
                    self.italic = prev_italic;
                    self.color_override = prev_color;
                    self.bg_override = prev_bg;
                }
            }
        }
    }

    fn push_text(&mut self, text: &str, is_code: bool) {
        if text.is_empty() {
            return;
        }

        let start = self.text.len();
        self.text.push_str(text);
        let len = text.len();

        let is_link = self.in_link.is_some();

        let font = if is_code {
            Font {
                family: self.font_mono.clone(),
                features: FontFeatures::default(),
                weight: FontWeight::default(),
                style: FontStyle::default(),
                fallbacks: None,
            }
        } else {
            Font {
                family: self.font_family.clone(),
                features: FontFeatures::default(),
                weight: if self.bold {
                    FontWeight::BOLD
                } else {
                    FontWeight::default()
                },
                style: if self.italic {
                    FontStyle::Italic
                } else {
                    FontStyle::Normal
                },
                fallbacks: None,
            }
        };

        let color = if is_link {
            self.link_color
        } else if let Some(c) = self.color_override {
            c
        } else {
            self.text_color
        };

        let underline = if is_link {
            Some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(self.link_color),
                wavy: false,
            })
        } else {
            None
        };

        let strikethrough = if self.strikethrough {
            Some(StrikethroughStyle {
                thickness: px(1.0),
                color: Some(self.text_color),
            })
        } else {
            None
        };

        let background_color = if is_code {
            Some(self.code_bg)
        } else {
            self.bg_override
        };

        self.runs.push(TextRun {
            len,
            font,
            color,
            background_color,
            underline,
            strikethrough,
        });

        if let Some(ref url) = self.in_link {
            self.links.push(LinkInfo {
                range: start..(start + len),
                url: url.clone(),
            });
        }
    }
}

// ShellDeck patch: SDPATCH-033 — link callbacks must survive long enough to
// be captured by every InteractiveText block produced from one Markdown tree.
// The previous boxed callback was only borrowed during rendering, so the
// renderer ignored it and called `cx.open_url` directly instead.
pub type LinkClickHandler = Rc<dyn Fn(&str, &mut Window, &mut App) + 'static>;

pub fn render_inlines(
    inlines: &[RichInline],
    base_size: Pixels,
    link_handler: Option<&LinkClickHandler>,
    element_id: Option<ElementId>,
) -> AnyElement {
    let theme = use_theme();
    let font_family = theme.tokens.font_family.clone();
    let font_mono = theme.tokens.font_mono.clone();
    let text_color = theme.tokens.foreground;
    let link_color = theme.tokens.primary;
    let code_bg = theme.tokens.muted.opacity(0.3);

    let flattened = InlineFlattener::new(font_family, font_mono, text_color, link_color, code_bg)
        .flatten(inlines);

    if flattened.text.is_empty() {
        return div().into_any_element();
    }

    let styled = StyledText::new(SharedString::from(flattened.text)).with_runs(flattened.runs);

    if !flattened.links.is_empty() && link_handler.is_some() {
        let id = element_id.unwrap_or_else(|| ElementId::Name("rich-text-inline".into()));
        let click_ranges: Vec<std::ops::Range<usize>> =
            flattened.links.iter().map(|l| l.range.clone()).collect();
        let link_urls: Vec<String> = flattened.links.iter().map(|l| l.url.clone()).collect();
        if let Some(handler) = link_handler.cloned() {
            let urls = link_urls;
            return div()
                .text_size(base_size)
                .line_height(relative(1.5))
                .child(InteractiveText::new(id, styled).on_click(
                    click_ranges,
                    move |idx, window, cx| {
                        if let Some(url) = urls.get(idx) {
                            handler(url, window, cx);
                        }
                    },
                ))
                .into_any_element();
        }
    }

    div()
        .text_size(base_size)
        .line_height(relative(1.5))
        .child(styled)
        .into_any_element()
}

pub fn render_inlines_with_handler(
    inlines: &[RichInline],
    base_size: Pixels,
    _link_urls: &[LinkInfo],
    on_link_click: &Option<LinkClickHandler>,
    element_id: Option<ElementId>,
) -> AnyElement {
    let theme = use_theme();
    let font_family = theme.tokens.font_family.clone();
    let font_mono = theme.tokens.font_mono.clone();
    let text_color = theme.tokens.foreground;
    let link_color = theme.tokens.primary;
    let code_bg = theme.tokens.muted.opacity(0.3);

    let flattened = InlineFlattener::new(font_family, font_mono, text_color, link_color, code_bg)
        .flatten(inlines);

    if flattened.text.is_empty() {
        return div().into_any_element();
    }

    let styled = StyledText::new(SharedString::from(flattened.text)).with_runs(flattened.runs);

    if !flattened.links.is_empty() {
        let id = element_id.unwrap_or_else(|| ElementId::Name("rich-text-inline".into()));
        let click_ranges: Vec<std::ops::Range<usize>> =
            flattened.links.iter().map(|l| l.range.clone()).collect();

        if let Some(handler) = on_link_click.clone() {
            let urls: Vec<String> = flattened.links.iter().map(|l| l.url.clone()).collect();
            return div()
                .text_size(base_size)
                .line_height(relative(1.5))
                .child(InteractiveText::new(id, styled).on_click(
                    click_ranges,
                    move |idx, window, cx| {
                        if let Some(url) = urls.get(idx) {
                            handler(url, window, cx);
                        }
                    },
                ))
                .into_any_element();
        }
        // ShellDeck patch: SDPATCH-035 — without an explicit host callback,
        // rich links remain styled text and cannot bypass confirmation by
        // calling the platform URL opener directly.
    }

    div()
        .text_size(base_size)
        .line_height(relative(1.5))
        .child(styled)
        .into_any_element()
}

pub fn render_blocks(
    blocks: &[RichBlock],
    base_size: Pixels,
    on_link_click: &Option<LinkClickHandler>,
    id_prefix: &str,
) -> Vec<AnyElement> {
    render_blocks_with_density(blocks, base_size, on_link_click, id_prefix, false)
}

// ShellDeck patch: SDPATCH-030 — thread prose needs 8 px block rhythm and no
// trailing document margin, while the existing renderer remains the default.
pub fn render_blocks_compact(
    blocks: &[RichBlock],
    base_size: Pixels,
    on_link_click: &Option<LinkClickHandler>,
    id_prefix: &str,
) -> Vec<AnyElement> {
    render_blocks_with_density(blocks, base_size, on_link_click, id_prefix, true)
}

fn render_blocks_with_density(
    blocks: &[RichBlock],
    base_size: Pixels,
    on_link_click: &Option<LinkClickHandler>,
    id_prefix: &str,
    compact: bool,
) -> Vec<AnyElement> {
    let mut elements = Vec::new();
    let mut block_idx = 0u32;

    for (position, block) in blocks.iter().enumerate() {
        let el = render_block(
            block,
            base_size,
            on_link_click,
            id_prefix,
            &mut block_idx,
            compact,
            position == 0,
            position + 1 == blocks.len(),
        );
        elements.push(el);
    }

    elements
}

// ShellDeck patch: SDPATCH-030 — compact conversation headings scale from the
// caller's body size instead of importing the 32/28/24/20 px document ramp.
// H6 meets the body size while H1 remains clearly dominant in a 480 px thread.
fn heading_size(level: u8, base_size: Pixels, compact: bool) -> Pixels {
    if !compact {
        return match level {
            1 => TextVariant::H1,
            2 => TextVariant::H2,
            3 => TextVariant::H3,
            4 => TextVariant::H4,
            5 => TextVariant::H5,
            _ => TextVariant::H6,
        }
        .size();
    }

    let scale = match level {
        1 => 1.44,
        2 => 1.32,
        3 => 1.20,
        4 => 1.12,
        5 => 1.06,
        _ => 1.0,
    };
    px(base_size.to_f64() as f32 * scale)
}

fn render_block(
    block: &RichBlock,
    base_size: Pixels,
    on_link_click: &Option<LinkClickHandler>,
    id_prefix: &str,
    block_idx: &mut u32,
    compact: bool,
    first: bool,
    last: bool,
) -> AnyElement {
    let theme = use_theme();
    *block_idx += 1;
    let idx = *block_idx;

    match block {
        RichBlock::Paragraph(inlines) => {
            let id = ElementId::Name(format!("{}-p-{}", id_prefix, idx).into());
            let el = render_inline_element(inlines, base_size, on_link_click, Some(id));
            div()
                .mb(if compact {
                    px(if last { 0.0 } else { 8.0 })
                } else {
                    px(12.0)
                })
                .child(el)
                .into_any_element()
        }

        RichBlock::Heading { level, content } => {
            let variant = match level {
                1 => TextVariant::H1,
                2 => TextVariant::H2,
                3 => TextVariant::H3,
                4 => TextVariant::H4,
                5 => TextVariant::H5,
                _ => TextVariant::H6,
            };
            // ShellDeck patch: SDPATCH-030 — compact headings use the thread
            // prototype's proportional type ramp, 10/4 rhythm and no indent
            // at the first/last edge. Document Markdown keeps TextVariant sizes.
            let size = heading_size(*level, base_size, compact);
            let weight = variant.weight();
            let id = ElementId::Name(format!("{}-h{}-{}", id_prefix, level, idx).into());

            let top_margin = if compact {
                px(if first { 0.0 } else { 10.0 })
            } else {
                match level {
                    1 => px(24.0),
                    2 => px(20.0),
                    3 => px(16.0),
                    _ => px(12.0),
                }
            };

            let el = render_inline_element(content, size, on_link_click, Some(id));
            div()
                .mt(top_margin)
                .mb(if compact {
                    px(if last { 0.0 } else { 4.0 })
                } else {
                    px(8.0)
                })
                .font_weight(weight)
                .child(el)
                .into_any_element()
        }

        RichBlock::CodeBlock { language, code } => {
            let mut cb = CodeBlock::new(code.clone())
                .show_line_numbers(true)
                .show_copy_button(true);
            if let Some(lang) = language {
                cb = cb.language(lang.clone());
            }
            div()
                .mb(if compact {
                    px(if last { 0.0 } else { 8.0 })
                } else {
                    px(12.0)
                })
                .child(cb)
                .into_any_element()
        }

        RichBlock::BlockQuote(inner_blocks) => {
            let children = render_blocks_with_density(
                inner_blocks,
                base_size,
                on_link_click,
                id_prefix,
                compact,
            );
            div()
                .mb(if compact {
                    px(if last { 0.0 } else { 8.0 })
                } else {
                    px(12.0)
                })
                .pl(px(if compact { 10.0 } else { 16.0 }))
                .border_l(px(if compact { 2.0 } else { 4.0 }))
                .border_color(theme.tokens.border)
                .bg(theme.tokens.muted.opacity(0.15))
                .py(px(if compact { 3.0 } else { 4.0 }))
                .text_color(theme.tokens.muted_foreground)
                .children(children)
                .into_any_element()
        }

        RichBlock::UnorderedList { items } => {
            let children = render_list_items(
                items,
                base_size,
                on_link_click,
                id_prefix,
                block_idx,
                false,
                1,
                compact,
            );
            div()
                .mb(if compact {
                    px(if last { 0.0 } else { 8.0 })
                } else {
                    px(12.0)
                })
                .children(children)
                .into_any_element()
        }

        RichBlock::OrderedList { start, items } => {
            let children = render_list_items(
                items,
                base_size,
                on_link_click,
                id_prefix,
                block_idx,
                true,
                *start,
                compact,
            );
            div()
                .mb(if compact {
                    px(if last { 0.0 } else { 8.0 })
                } else {
                    px(12.0)
                })
                .children(children)
                .into_any_element()
        }

        RichBlock::Table {
            headers,
            alignments,
            rows,
        } => render_table(
            headers,
            alignments,
            rows,
            base_size,
            on_link_click,
            id_prefix,
            block_idx,
            compact,
            last,
        ),

        RichBlock::HorizontalRule => div()
            .mt(px(if compact && first {
                0.0
            } else if compact {
                8.0
            } else {
                16.0
            }))
            .mb(px(if compact && last {
                0.0
            } else if compact {
                8.0
            } else {
                16.0
            }))
            .child(Separator::new())
            .into_any_element(),

        RichBlock::Image { alt: _, url } => div()
            .mb(if compact {
                px(if last { 0.0 } else { 8.0 })
            } else {
                px(12.0)
            })
            .child(img(SharedString::from(url.clone())).max_w(px(600.0)))
            .into_any_element(),
    }
}

fn render_inline_element(
    inlines: &[RichInline],
    base_size: Pixels,
    on_link_click: &Option<LinkClickHandler>,
    element_id: Option<ElementId>,
) -> AnyElement {
    let theme = use_theme();
    let font_family = theme.tokens.font_family.clone();
    let font_mono = theme.tokens.font_mono.clone();
    let text_color = theme.tokens.foreground;
    let link_color = theme.tokens.primary;
    let code_bg = theme.tokens.muted.opacity(0.3);

    let flattened = InlineFlattener::new(font_family, font_mono, text_color, link_color, code_bg)
        .flatten(inlines);

    if flattened.text.is_empty() {
        return div().into_any_element();
    }

    let styled = StyledText::new(SharedString::from(flattened.text)).with_runs(flattened.runs);

    if !flattened.links.is_empty() {
        let id = element_id.unwrap_or_else(|| ElementId::Name("rich-inline".into()));
        let click_ranges: Vec<std::ops::Range<usize>> =
            flattened.links.iter().map(|l| l.range.clone()).collect();
        let urls: Vec<String> = flattened.links.iter().map(|l| l.url.clone()).collect();

        if let Some(handler) = on_link_click.clone() {
            return div()
                .text_size(base_size)
                .line_height(relative(1.5))
                .child(InteractiveText::new(id, styled).on_click(
                    click_ranges,
                    move |idx, window, cx| {
                        if let Some(url) = urls.get(idx) {
                            handler(url, window, cx);
                        }
                    },
                ))
                .into_any_element();
        }
        // ShellDeck patch: SDPATCH-035 — passive is the safe default for
        // parsed remote content. A caller must opt into interaction and own
        // the confirmation policy via `on_link_click`.
    }

    div()
        .text_size(base_size)
        .line_height(relative(1.5))
        .child(styled)
        .into_any_element()
}

fn render_list_items(
    items: &[ListItem],
    base_size: Pixels,
    on_link_click: &Option<LinkClickHandler>,
    id_prefix: &str,
    block_idx: &mut u32,
    ordered: bool,
    start: u64,
    compact: bool,
) -> Vec<AnyElement> {
    let theme = use_theme();
    let mut elements = Vec::new();

    for (i, item) in items.iter().enumerate() {
        *block_idx += 1;
        let idx = *block_idx;

        // ShellDeck patch: SDPATCH-031 — task-list markers are real disabled
        // checkboxes instead of the plain-text strings `[x]` and `[ ]`.
        let marker = if let Some(checked) = item.checked {
            Checkbox::new(ElementId::Name(
                format!("{}-task-{}", id_prefix, idx).into(),
            ))
            .size(CheckboxSize::Sm)
            .checked(checked)
            .disabled(true)
            .into_any_element()
        } else {
            div()
                .text_size(base_size)
                .text_color(theme.tokens.muted_foreground)
                .child(if ordered {
                    SharedString::from(format!("{}. ", start + i as u64))
                } else {
                    SharedString::from("\u{2022} ")
                })
                .into_any_element()
        };

        let id = ElementId::Name(format!("{}-li-{}", id_prefix, idx).into());
        let content_el = render_inline_element(&item.content, base_size, on_link_click, Some(id));

        let row = div()
            .flex()
            .flex_row()
            .pl(px(20.0))
            // ShellDeck patch: SDPATCH-030 — list rows in chat use the
            // prototype's tighter 2 px cadence.
            .mb(px(if compact { 2.0 } else { 4.0 }))
            .child(div().flex_shrink_0().w(px(24.0)).child(marker))
            // ShellDeck patch: SDPATCH-027 — list text must be the flex child
            // that shrinks so GPUI receives a definite width and wraps it.
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .whitespace_normal()
                    .child(content_el),
            );

        elements.push(row.into_any_element());

        if !item.children.is_empty() {
            let children = render_list_items(
                &item.children,
                base_size,
                on_link_click,
                id_prefix,
                block_idx,
                false,
                1,
                compact,
            );
            elements.push(div().pl(px(20.0)).children(children).into_any_element());
        }
    }

    elements
}

fn render_table(
    headers: &[Vec<RichInline>],
    alignments: &[TableAlignment],
    rows: &[Vec<Vec<RichInline>>],
    base_size: Pixels,
    on_link_click: &Option<LinkClickHandler>,
    id_prefix: &str,
    block_idx: &mut u32,
    compact: bool,
    last: bool,
) -> AnyElement {
    let theme = use_theme();
    let col_count = headers.len();
    if col_count == 0 {
        return div().into_any_element();
    }

    // ShellDeck patch: SDPATCH-030 — compact tables participate in the same
    // 8 px block cadence and leave no tail before thread metadata.
    let mut table = div()
        .mb(if compact {
            px(if last { 0.0 } else { 8.0 })
        } else {
            px(12.0)
        })
        .w_full()
        .rounded(theme.tokens.radius_sm)
        .border_1()
        .border_color(theme.tokens.border)
        .overflow_hidden();

    *block_idx += 1;
    let header_row = {
        let mut row = div()
            .flex()
            .flex_row()
            .bg(theme.tokens.muted.opacity(0.3))
            .border_b_1()
            .border_color(theme.tokens.border);

        for (ci, header) in headers.iter().enumerate() {
            *block_idx += 1;
            let id = ElementId::Name(format!("{}-th-{}", id_prefix, *block_idx).into());
            let el = render_inline_element(header, base_size, on_link_click, Some(id));
            let mut cell = div()
                .flex_1()
                // ShellDeck patch: SDPATCH-027 — table headers need a
                // shrinkable width before StyledText can wrap inside them.
                .min_w(px(0.0))
                .whitespace_normal()
                .px(px(12.0))
                .py(px(8.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(base_size)
                .child(el);

            if ci < alignments.len() {
                cell = match &alignments[ci] {
                    TableAlignment::Center => cell.items_center(),
                    TableAlignment::Right => cell.items_end(),
                    TableAlignment::Left => cell,
                };
            }

            row = row.child(cell);
        }
        row
    };
    table = table.child(header_row);

    for (ri, row_data) in rows.iter().enumerate() {
        *block_idx += 1;
        let mut row = div()
            .flex()
            .flex_row()
            .when(ri < rows.len() - 1, |this| {
                this.border_b_1().border_color(theme.tokens.border)
            })
            .when(ri % 2 == 1, |this| this.bg(theme.tokens.muted.opacity(0.1)));

        for (ci, cell_data) in row_data.iter().enumerate() {
            *block_idx += 1;
            let id = ElementId::Name(format!("{}-td-{}", id_prefix, *block_idx).into());
            let el = render_inline_element(cell_data, base_size, on_link_click, Some(id));
            let mut cell = div()
                .flex_1()
                // ShellDeck patch: SDPATCH-027 — body cells follow the same
                // wrapping contract as headers instead of bleeding or clipping.
                .min_w(px(0.0))
                .whitespace_normal()
                .px(px(12.0))
                .py(px(6.0))
                .text_size(base_size)
                .child(el);

            if ci < alignments.len() {
                cell = match &alignments[ci] {
                    TableAlignment::Center => cell.items_center(),
                    TableAlignment::Right => cell.items_end(),
                    TableAlignment::Left => cell,
                };
            }

            row = row.child(cell);
        }
        table = table.child(row);
    }

    table.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::heading_size;
    use gpui::px;

    // SDTEST-1621
    #[test]
    fn compact_heading_scale_is_relative_and_document_sizes_are_unchanged() {
        let base = px(12.5);
        let compact = [
            heading_size(1, base, true),
            heading_size(2, base, true),
            heading_size(3, base, true),
            heading_size(4, base, true),
            heading_size(5, base, true),
            heading_size(6, base, true),
        ];
        let expected = [18.0, 16.5, 15.0, 14.0, 13.25, 12.5];
        for (actual, expected) in compact.into_iter().zip(expected) {
            assert!((actual.to_f64() as f32 - expected).abs() < 0.001);
        }

        assert_eq!(heading_size(1, base, false), px(32.0));
        assert_eq!(heading_size(4, base, false), px(20.0));
        assert_eq!(heading_size(6, base, false), px(16.0));
    }
}

// ShellDeck patch: SDPATCH-040 — paint chosen tokens inside already-parsed
// inline text.
//
// A mention has to read as a mention once the turn is sent, not only while it
// is typed. Markdown has no notion of one, and inventing a syntax for it would
// mean the model receives that syntax too. So the colour is applied to the
// parsed tree instead: the source stays exactly what the user wrote and what
// was sent, and only the rendering differs.
//
// Tokens are matched longest-first and never overlap, mirroring
// `shelldeck_core::ai::mentions::mention_spans`.

/// Wrap every occurrence of `tokens` in `blocks` with a coloured span.
pub fn highlight_tokens_in_blocks(
    blocks: &mut [RichBlock],
    tokens: &[SharedString],
    color: Hsla,
    background: Option<Hsla>,
) {
    if tokens.is_empty() {
        return;
    }
    let mut ordered: Vec<&SharedString> = tokens.iter().filter(|t| !t.is_empty()).collect();
    ordered.sort_by_key(|token| std::cmp::Reverse(token.len()));
    for block in blocks {
        highlight_block(block, &ordered, color, background);
    }
}

fn highlight_block(
    block: &mut RichBlock,
    tokens: &[&SharedString],
    color: Hsla,
    background: Option<Hsla>,
) {
    match block {
        RichBlock::Paragraph(inlines) | RichBlock::Heading { content: inlines, .. } => {
            highlight_inlines(inlines, tokens, color, background)
        }
        RichBlock::BlockQuote(children) => {
            for child in children {
                highlight_block(child, tokens, color, background);
            }
        }
        RichBlock::OrderedList { items, .. } | RichBlock::UnorderedList { items } => {
            for item in items {
                highlight_list_item(item, tokens, color, background);
            }
        }
        RichBlock::Table { headers, rows, .. } => {
            for cell in headers {
                highlight_inlines(cell, tokens, color, background);
            }
            for row in rows {
                for cell in row {
                    highlight_inlines(cell, tokens, color, background);
                }
            }
        }
        // Code keeps its own styling: a token inside a fenced block is a
        // literal the user typed, not a reference.
        RichBlock::CodeBlock { .. } | RichBlock::HorizontalRule | RichBlock::Image { .. } => {}
    }
}

fn highlight_list_item(
    item: &mut ListItem,
    tokens: &[&SharedString],
    color: Hsla,
    background: Option<Hsla>,
) {
    highlight_inlines(&mut item.content, tokens, color, background);
    for child in &mut item.children {
        highlight_list_item(child, tokens, color, background);
    }
}

fn highlight_inlines(
    inlines: &mut Vec<RichInline>,
    tokens: &[&SharedString],
    color: Hsla,
    background: Option<Hsla>,
) {
    let mut out: Vec<RichInline> = Vec::with_capacity(inlines.len());
    for inline in inlines.drain(..) {
        match inline {
            RichInline::Text(text) => out.extend(split_text(&text, tokens, color, background)),
            RichInline::Bold(mut children) => {
                highlight_inlines(&mut children, tokens, color, background);
                out.push(RichInline::Bold(children));
            }
            RichInline::Italic(mut children) => {
                highlight_inlines(&mut children, tokens, color, background);
                out.push(RichInline::Italic(children));
            }
            RichInline::Strikethrough(mut children) => {
                highlight_inlines(&mut children, tokens, color, background);
                out.push(RichInline::Strikethrough(children));
            }
            RichInline::Styled {
                mut children,
                color: own,
                background_color,
                bold,
                italic,
                font_size,
            } => {
                // An explicit colour already set by the source wins.
                if own.is_none() {
                    highlight_inlines(&mut children, tokens, color, background);
                }
                out.push(RichInline::Styled {
                    children,
                    color: own,
                    background_color,
                    bold,
                    italic,
                    font_size,
                });
            }
            // Link text is already coloured and clickable; code is literal.
            other => out.push(other),
        }
    }
    *inlines = out;
}

fn split_text(
    text: &str,
    tokens: &[&SharedString],
    color: Hsla,
    background: Option<Hsla>,
) -> Vec<RichInline> {
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for token in tokens {
        let mut from = 0usize;
        while let Some(found) = text[from..].find(token.as_ref()) {
            let start = from + found;
            let end = start + token.len();
            from = end;
            if start > 0
                && !text[..start]
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace)
            {
                continue;
            }
            if spans.iter().any(|(s, e)| start < *e && *s < end) {
                continue;
            }
            spans.push((start, end));
        }
    }
    if spans.is_empty() {
        return vec![RichInline::Text(text.to_string())];
    }
    spans.sort_unstable();
    let mut out = Vec::new();
    let mut cursor = 0usize;
    for (start, end) in spans {
        if start > cursor {
            out.push(RichInline::Text(text[cursor..start].to_string()));
        }
        out.push(RichInline::Styled {
            children: vec![RichInline::Text(text[start..end].to_string())],
            color: Some(color),
            background_color: background,
            bold: Some(true),
            italic: None,
            font_size: None,
        });
        cursor = end;
    }
    if cursor < text.len() {
        out.push(RichInline::Text(text[cursor..].to_string()));
    }
    out
}
