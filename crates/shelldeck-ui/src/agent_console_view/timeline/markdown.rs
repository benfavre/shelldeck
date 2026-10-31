//! Long prose is indexed by complete Markdown blocks, preserving document-wide links.
use super::*;
use adabraka_ui::display::{
    markdown::parse_markdown_with_urls,
    rich_text::{render_blocks_compact_range, RichBlock},
};
use std::cell::Cell;

const BLOCKS_PER_PART: usize = 8;
type Stamp = (usize, u64);
type Parsed = Rc<Vec<RichBlock>>;
#[derive(Default)]
pub(super) struct MarkdownRows {
    plans: HashMap<Uuid, (Stamp, usize)>,
    parsed: RefCell<HashMap<Uuid, (Stamp, Parsed, u64)>>,
    seen: Cell<u64>,
    #[cfg(test)]
    pub(super) rendered_blocks: Cell<usize>,
}
impl MarkdownRows {
    pub(super) fn parts(&mut self, message: &AgentMessage, epoch: u64) -> Option<usize> {
        if message.role != AgentMessageRole::Agent || message.text.len() < 4096 {
            self.plans.remove(&message.id);
            self.parsed.borrow_mut().remove(&message.id);
            return None;
        }
        let stamp = (message.text.len(), epoch);
        let parts = if let Some((_, parts)) =
            self.plans.get(&message.id).filter(|(key, _)| *key == stamp)
        {
            *parts
        } else {
            let parts = if transcript::encoded_spans(&message.text).is_empty() {
                let blocks = self.blocks(message, epoch);
                if blocks.len() > BLOCKS_PER_PART * 2 {
                    blocks.len().div_ceil(BLOCKS_PER_PART)
                } else {
                    0
                }
            } else {
                0
            };
            self.plans.insert(message.id, (stamp, parts));
            parts
        };
        (parts > 0).then_some(parts)
    }
    fn blocks(&self, message: &AgentMessage, epoch: u64) -> Parsed {
        let stamp = (message.text.len(), epoch);
        let seen = self.seen.get().wrapping_add(1);
        self.seen.set(seen);
        let mut cache = self.parsed.borrow_mut();
        if let Some((_, blocks, touched)) = cache
            .get_mut(&message.id)
            .filter(|(key, _, _)| *key == stamp)
        {
            *touched = seen;
            return blocks.clone();
        }
        let blocks = Rc::new(parse_markdown_with_urls(&message.text));
        if cache.len() >= 4 && !cache.contains_key(&message.id) {
            if let Some(key) = cache
                .iter()
                .min_by_key(|(_, (_, _, touched))| *touched)
                .map(|(key, _)| *key)
            {
                cache.remove(&key);
            }
        }
        cache.insert(message.id, (stamp, blocks.clone(), seen));
        blocks
    }
    pub(super) fn retain(&mut self, session: &AgentSession) {
        let ids = session
            .messages
            .iter()
            .map(|message| message.id)
            .collect::<HashSet<_>>();
        self.plans.retain(|id, _| ids.contains(id));
        self.parsed.borrow_mut().retain(|id, _| ids.contains(id));
    }
}

impl AgentConsoleView {
    pub(super) fn render_message_part(
        &self,
        message: &AgentMessage,
        part: usize,
        epoch: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let blocks = self.timeline.markdown.blocks(message, epoch);
        let start = (part * BLOCKS_PER_PART).min(blocks.len());
        let end = (start + BLOCKS_PER_PART).min(blocks.len());
        #[cfg(test)]
        {
            self.rendered_transcripts
                .set(self.rendered_transcripts.get() + 1);
            let count = &self.timeline.markdown.rendered_blocks;
            count.set(count.get() + end - start);
        }
        let key = format!("agent-message-{}-part-{part}", message.id);
        let theme = adabraka_ui::theme::use_theme();
        let mut row = div()
            .id(SharedString::from(key.clone()))
            .debug_selector(move || key.clone())
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .overflow_hidden();
        if part == 0 {
            row = row.pt(px(10.0)).child(
                div()
                    .mb(px(4.0))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ShellDeckColors::text_muted())
                    .child(
                        message
                            .provider
                            .unwrap_or(self.displayed_context(cx).provider)
                            .display_name()
                            .to_string(),
                    ),
            );
        }
        row = row.child(
            div()
                .flex()
                .flex_col()
                .w_full()
                .min_w_0()
                .font_family(theme.tokens.font_family.clone())
                .text_color(theme.tokens.foreground)
                .whitespace_normal()
                .children(render_blocks_compact_range(
                    &blocks,
                    start..end,
                    px(14.0).to_pixels(window.rem_size()),
                    &None,
                    &format!("message-{}-{part}", message.id),
                )),
        );
        if end == blocks.len() {
            row = row
                .child(self.render_message_actions(message, cx))
                .pb(px(10.0));
        }
        row.into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // SDTEST-2881 / SDUC-475, SDUC-564 — chunks share the full-document parser and bounded AST cache.
    #[::core::prelude::v1::test]
    fn prose_block_plans_preserve_references_structures_and_bounded_source_cache() {
        let source=format!("[reference link][target]\n\n{}\n```rust\nfn main() {{}}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\n[target]: https://example.test/document\n",
            "A complete **paragraph** remains one indivisible block of readable source content.\n\n".repeat(100));
        let mut message = AgentMessage {
            id: Uuid::new_v4(),
            sequence: 1,
            turn_id: None,
            role: AgentMessageRole::Agent,
            provider: Some(AgentProvider::Codex),
            text: source,
            at_ms: 1,
        };
        let mut rows = MarkdownRows::default();
        let count = rows.parts(&message, 1).unwrap();
        let blocks = rows.blocks(&message, 1);
        assert_eq!(count, blocks.len().div_ceil(BLOCKS_PER_PART));
        assert!(
            matches!(&blocks[0],RichBlock::Paragraph(inlines) if inlines.iter().any(|inline|matches!(inline,adabraka_ui::display::rich_text::RichInline::Link {url,..} if url=="https://example.test/document")))
        );
        assert!(blocks.iter().any(
            |block| matches!(block,RichBlock::CodeBlock {code,..} if code.contains("fn main() {}"))
        ));
        assert!(blocks
            .iter()
            .any(|block| matches!(block,RichBlock::Table {rows,..} if rows.len()==1)));
        for _ in 0..12 {
            message.id = Uuid::new_v4();
            assert!(rows.parts(&message, 1).is_some());
        }
        assert!(
            rows.parsed.borrow().len() <= 4,
            "offscreen documents cannot retain unbounded parsed copies"
        );
        message.text = format!("Before\n{}\nAfter", "aB3+/xY9".repeat(2000));
        assert!(
            rows.parts(&message, 2).is_none(),
            "encoded disclosure must retain its original fold/reveal path"
        );
    }
}
