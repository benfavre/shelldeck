#[cfg(feature = "markdown")]
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use gpui::*;
#[cfg(feature = "markdown")]
use once_cell::sync::Lazy;
#[cfg(feature = "markdown")]
use regex::Regex;

use crate::display::rich_text::LinkClickHandler;
#[cfg(feature = "markdown")]
use crate::display::rich_text::{
    render_blocks, render_blocks_compact, ListItem, RichBlock, RichInline, TableAlignment,
};
#[cfg(feature = "markdown")]
use crate::theme::use_theme;

#[derive(IntoElement)]
pub struct Markdown {
    base: Div,
    source: SharedString,
    base_font_size: Option<Pixels>,
    on_link_click: Option<LinkClickHandler>,
    // ShellDeck patch: SDPATCH-030 — chat-sized Markdown follows compact
    // prose rhythm and does not leave document margins after its last block.
    compact: bool,
}

impl Markdown {
    pub fn new(source: impl Into<SharedString>) -> Self {
        Self {
            base: div(),
            source: source.into(),
            base_font_size: None,
            on_link_click: None,
            compact: false,
        }
    }

    pub fn base_font_size(mut self, size: Pixels) -> Self {
        self.base_font_size = Some(size);
        self
    }

    // ShellDeck patch: SDPATCH-030 — opt into the thread/note block spacing
    // without changing Markdown's document-oriented default rendering.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    pub fn on_link_click(
        mut self,
        handler: impl Fn(&str, &mut Window, &mut App) + 'static,
    ) -> Self {
        // ShellDeck patch: SDPATCH-033 — shared by every interactive inline
        // emitted from this Markdown document.
        self.on_link_click = Some(std::rc::Rc::new(handler));
        self
    }
}

#[cfg(feature = "markdown")]
fn heading_level_to_u8(level: &HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

#[cfg(feature = "markdown")]
fn inlines_to_plain_text(inlines: &[RichInline]) -> String {
    let mut out = String::new();
    for inline in inlines {
        match inline {
            RichInline::Text(s) => out.push_str(s),
            RichInline::Bold(children)
            | RichInline::Italic(children)
            | RichInline::Strikethrough(children) => {
                out.push_str(&inlines_to_plain_text(children));
            }
            RichInline::Code(s) => out.push_str(s),
            RichInline::Link { text, .. } => {
                out.push_str(&inlines_to_plain_text(text));
            }
            RichInline::Image { alt, .. } => out.push_str(alt),
            RichInline::LineBreak => out.push('\n'),
            RichInline::Html(_) => {}
            RichInline::Styled { children, .. } => {
                out.push_str(&inlines_to_plain_text(children));
            }
        }
    }
    out
}

#[cfg(feature = "markdown")]
impl RenderOnce for Markdown {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let theme = use_theme();
        let base_size = self.base_font_size.unwrap_or(px(14.0));

        let blocks = parse_markdown_with_urls(&self.source);
        // ShellDeck patch: SDPATCH-030 — select compact margins only for
        // explicit chat consumers; HTML and regular Markdown stay unchanged.
        let elements = if self.compact {
            render_blocks_compact(&blocks, base_size, &self.on_link_click, "md")
        } else {
            render_blocks(&blocks, base_size, &self.on_link_click, "md")
        };

        self.base
            .flex()
            .flex_col()
            .font_family(theme.tokens.font_family.clone())
            .text_color(theme.tokens.foreground)
            .children(elements)
    }
}

#[cfg(not(feature = "markdown"))]
impl RenderOnce for Markdown {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let _ = &self.source;
        self.base
            .child("Enable the 'markdown' feature to render markdown content.")
    }
}

impl Styled for Markdown {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

#[cfg(feature = "markdown")]
struct ListState {
    ordered: bool,
    start: u64,
    items: Vec<ListItem>,
    current_item_inlines: Vec<RichInline>,
    current_item_checked: Option<bool>,
}

#[cfg(feature = "markdown")]
struct TableState {
    headers: Vec<Vec<RichInline>>,
    alignments: Vec<TableAlignment>,
    rows: Vec<Vec<Vec<RichInline>>>,
    current_row: Vec<Vec<RichInline>>,
    in_head: bool,
}

#[cfg(feature = "markdown")]
fn parse_markdown_with_urls(source: &str) -> Vec<RichBlock> {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(source, options);
    let events: Vec<Event> = parser.collect();

    let mut builder = UrlTrackingBlockBuilder::new();
    builder.build(&events);
    builder.blocks
}

#[cfg(feature = "markdown")]
struct UrlTrackingBlockBuilder {
    blocks: Vec<RichBlock>,
    inline_stack: Vec<Vec<RichInline>>,
    list_stack: Vec<ListState>,
    blockquote_depth: usize,
    blockquote_blocks: Vec<Vec<RichBlock>>,
    table_state: Option<TableState>,
    current_heading_level: Option<u8>,
    in_code_block: bool,
    code_block_lang: Option<String>,
    code_block_content: String,
    url_stack: Vec<String>,
}

#[cfg(feature = "markdown")]
impl UrlTrackingBlockBuilder {
    fn new() -> Self {
        Self {
            blocks: Vec::new(),
            inline_stack: Vec::new(),
            list_stack: Vec::new(),
            blockquote_depth: 0,
            blockquote_blocks: Vec::new(),
            table_state: None,
            current_heading_level: None,
            in_code_block: false,
            code_block_lang: None,
            code_block_content: String::new(),
            url_stack: Vec::new(),
        }
    }

    fn build(&mut self, events: &[Event]) {
        for event in events {
            self.process_event(event);
        }
    }

    fn process_event(&mut self, event: &Event) {
        match event {
            Event::Start(tag) => self.start_tag(tag),
            Event::End(tag) => self.end_tag(tag),
            Event::Text(text) => self.text(text),
            Event::Code(code) => self.push_inline(RichInline::Code(code.to_string())),
            Event::SoftBreak => self.push_inline(RichInline::Text(" ".to_string())),
            Event::HardBreak => self.push_inline(RichInline::LineBreak),
            Event::Rule => self.push_block(RichBlock::HorizontalRule),
            Event::Html(html) => self.push_inline(RichInline::Html(html.to_string())),
            Event::TaskListMarker(checked) => {
                if let Some(list) = self.list_stack.last_mut() {
                    list.current_item_checked = Some(*checked);
                }
            }
            _ => {}
        }
    }

    fn start_tag(&mut self, tag: &Tag) {
        match tag {
            Tag::Paragraph => {
                self.inline_stack.push(Vec::new());
            }
            Tag::Heading { level, .. } => {
                self.current_heading_level = Some(heading_level_to_u8(level));
                self.inline_stack.push(Vec::new());
            }
            Tag::BlockQuote(_) => {
                self.blockquote_depth += 1;
                self.blockquote_blocks.push(Vec::new());
            }
            Tag::CodeBlock(kind) => {
                self.in_code_block = true;
                self.code_block_content.clear();
                match kind {
                    pulldown_cmark::CodeBlockKind::Fenced(lang) => {
                        let lang_str = lang.to_string();
                        self.code_block_lang = if lang_str.is_empty() {
                            None
                        } else {
                            Some(lang_str)
                        };
                    }
                    pulldown_cmark::CodeBlockKind::Indented => {
                        self.code_block_lang = None;
                    }
                }
            }
            Tag::List(start) => {
                self.list_stack.push(ListState {
                    ordered: start.is_some(),
                    start: start.unwrap_or(1),
                    items: Vec::new(),
                    current_item_inlines: Vec::new(),
                    current_item_checked: None,
                });
            }
            Tag::Item => {
                if let Some(list) = self.list_stack.last_mut() {
                    list.current_item_inlines.clear();
                    list.current_item_checked = None;
                }
                self.inline_stack.push(Vec::new());
            }
            Tag::Strong => {
                self.inline_stack.push(Vec::new());
            }
            Tag::Emphasis => {
                self.inline_stack.push(Vec::new());
            }
            Tag::Strikethrough => {
                self.inline_stack.push(Vec::new());
            }
            Tag::Link { dest_url, .. } => {
                self.url_stack.push(dest_url.to_string());
                self.inline_stack.push(Vec::new());
            }
            Tag::Image { dest_url, .. } => {
                self.url_stack.push(dest_url.to_string());
                self.inline_stack.push(Vec::new());
            }
            Tag::Table(alignments) => {
                self.table_state = Some(TableState {
                    headers: Vec::new(),
                    alignments: alignments
                        .iter()
                        .map(|a| match a {
                            pulldown_cmark::Alignment::Left => TableAlignment::Left,
                            pulldown_cmark::Alignment::Center => TableAlignment::Center,
                            pulldown_cmark::Alignment::Right => TableAlignment::Right,
                            pulldown_cmark::Alignment::None => TableAlignment::Left,
                        })
                        .collect(),
                    rows: Vec::new(),
                    current_row: Vec::new(),
                    in_head: false,
                });
            }
            Tag::TableHead => {
                if let Some(ref mut ts) = self.table_state {
                    ts.in_head = true;
                    ts.current_row.clear();
                }
            }
            Tag::TableRow => {
                if let Some(ref mut ts) = self.table_state {
                    ts.current_row.clear();
                }
            }
            Tag::TableCell => {
                self.inline_stack.push(Vec::new());
            }
            _ => {}
        }
    }

    fn end_tag(&mut self, tag: &TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                let inlines = self.inline_stack.pop().unwrap_or_default();
                self.push_block(RichBlock::Paragraph(inlines));
            }
            TagEnd::Heading(_level) => {
                let inlines = self.inline_stack.pop().unwrap_or_default();
                let lvl = self.current_heading_level.take().unwrap_or(1);
                self.push_block(RichBlock::Heading {
                    level: lvl,
                    content: inlines,
                });
            }
            TagEnd::BlockQuote(_) => {
                self.blockquote_depth -= 1;
                let inner = self.blockquote_blocks.pop().unwrap_or_default();
                self.push_block(RichBlock::BlockQuote(inner));
            }
            TagEnd::CodeBlock => {
                self.in_code_block = false;
                let code = std::mem::take(&mut self.code_block_content);
                let lang = self.code_block_lang.take();
                self.push_block(RichBlock::CodeBlock {
                    language: lang,
                    code,
                });
            }
            TagEnd::List(_ordered) => {
                if let Some(list) = self.list_stack.pop() {
                    let block = if list.ordered {
                        RichBlock::OrderedList {
                            start: list.start,
                            items: list.items,
                        }
                    } else {
                        RichBlock::UnorderedList { items: list.items }
                    };
                    self.push_block(block);
                }
            }
            TagEnd::Item => {
                let inlines = self.inline_stack.pop().unwrap_or_default();
                if let Some(list) = self.list_stack.last_mut() {
                    list.items.push(ListItem {
                        checked: list.current_item_checked,
                        content: inlines,
                        children: Vec::new(),
                    });
                }
            }
            TagEnd::Strong => {
                let children = self.inline_stack.pop().unwrap_or_default();
                self.push_inline(RichInline::Bold(children));
            }
            TagEnd::Emphasis => {
                let children = self.inline_stack.pop().unwrap_or_default();
                self.push_inline(RichInline::Italic(children));
            }
            TagEnd::Strikethrough => {
                let children = self.inline_stack.pop().unwrap_or_default();
                self.push_inline(RichInline::Strikethrough(children));
            }
            TagEnd::Link => {
                let children = self.inline_stack.pop().unwrap_or_default();
                let url = self.url_stack.pop().unwrap_or_default();
                self.push_inline(RichInline::Link {
                    text: children,
                    url,
                });
            }
            TagEnd::Image => {
                let alt_inlines = self.inline_stack.pop().unwrap_or_default();
                let alt = inlines_to_plain_text(&alt_inlines);
                let url = self.url_stack.pop().unwrap_or_default();
                self.push_block(RichBlock::Image { alt, url });
            }
            TagEnd::Table => {
                if let Some(ts) = self.table_state.take() {
                    self.push_block(RichBlock::Table {
                        headers: ts.headers,
                        alignments: ts.alignments,
                        rows: ts.rows,
                    });
                }
            }
            TagEnd::TableHead => {
                if let Some(ref mut ts) = self.table_state {
                    ts.headers = std::mem::take(&mut ts.current_row);
                    ts.in_head = false;
                }
            }
            TagEnd::TableRow => {
                if let Some(ref mut ts) = self.table_state {
                    if !ts.in_head {
                        let row = std::mem::take(&mut ts.current_row);
                        ts.rows.push(row);
                    }
                }
            }
            TagEnd::TableCell => {
                let inlines = self.inline_stack.pop().unwrap_or_default();
                if let Some(ref mut ts) = self.table_state {
                    ts.current_row.push(inlines);
                }
            }
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        if self.in_code_block {
            self.code_block_content.push_str(text);
            return;
        }
        // ShellDeck patch: SDPATCH-034 — chat APIs mostly return bare URLs,
        // not Markdown `[labels](destinations)`. Promote http(s) spans to the
        // same RichInline::Link used by authored Markdown so styling, cursor
        // and the caller's confirmation handler are identical.
        static BARE_URL: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r#"https?://[^\s<>{}\[\]\"']+"#).expect("valid bare URL regex")
        });
        if !self.url_stack.is_empty() {
            self.push_inline(RichInline::Text(text.to_string()));
            return;
        }

        let mut cursor = 0;
        for found in BARE_URL.find_iter(text) {
            if found.start() > cursor {
                self.push_inline(RichInline::Text(text[cursor..found.start()].to_string()));
            }
            let raw = found.as_str();
            let url = raw.trim_end_matches(['.', ',', ';', ':', '!', '?', ')']);
            if url.is_empty() {
                self.push_inline(RichInline::Text(raw.to_string()));
            } else {
                self.push_inline(RichInline::Link {
                    text: vec![RichInline::Text(url.to_string())],
                    url: url.to_string(),
                });
                if url.len() < raw.len() {
                    self.push_inline(RichInline::Text(raw[url.len()..].to_string()));
                }
            }
            cursor = found.end();
        }
        if cursor < text.len() {
            self.push_inline(RichInline::Text(text[cursor..].to_string()));
        } else if cursor == 0 {
            self.push_inline(RichInline::Text(text.to_string()));
        }
    }

    fn push_inline(&mut self, inline: RichInline) {
        if let Some(stack) = self.inline_stack.last_mut() {
            stack.push(inline);
        }
    }

    fn push_block(&mut self, block: RichBlock) {
        if self.blockquote_depth > 0 {
            if let Some(blocks) = self.blockquote_blocks.last_mut() {
                blocks.push(block);
                return;
            }
        }
        self.blocks.push(block);
    }
}

#[cfg(all(test, feature = "markdown"))]
mod tests {
    use super::{parse_markdown_with_urls, RichBlock, RichInline};

    #[test]
    fn bare_http_url_is_promoted_without_trailing_punctuation() {
        let blocks = parse_markdown_with_urls(
            "Consulte https://manage.inklura.fr/tickets/42, puis réponds.",
        );
        let RichBlock::Paragraph(inlines) = &blocks[0] else {
            panic!("expected paragraph");
        };
        assert!(inlines.iter().any(|inline| matches!(
            inline,
            RichInline::Link { url, .. }
                if url == "https://manage.inklura.fr/tickets/42"
        )));
        assert!(inlines
            .iter()
            .any(|inline| matches!(inline, RichInline::Text(text) if text.starts_with(','))));
    }

    #[test]
    fn explicit_markdown_link_is_not_linkified_twice() {
        let blocks = parse_markdown_with_urls("[Manage](https://manage.inklura.fr)");
        let RichBlock::Paragraph(inlines) = &blocks[0] else {
            panic!("expected paragraph");
        };
        assert_eq!(
            inlines
                .iter()
                .filter(|inline| matches!(inline, RichInline::Link { .. }))
                .count(),
            1
        );
    }
}
