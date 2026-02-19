use gpui::Hsla;
use ropey::Rope;
use std::ops::Range;
use tree_sitter::{InputEdit, Parser, Point, Query, QueryCursor, StreamingIterator, Tree};

use super::EditorLanguage;
use crate::theme::ShellDeckColors;

/// A highlight span within a single line.
#[derive(Debug, Clone)]
pub struct HighlightSpan {
    /// Byte range within the line (relative to line start).
    pub range: Range<usize>,
    pub color: Hsla,
    pub bold: bool,
    pub italic: bool,
}

/// Cached highlights for a viewport range.
struct CachedHighlights {
    start_line: usize,
    end_line: usize,
    /// Highlights per line (index 0 = start_line).
    lines: Vec<Vec<HighlightSpan>>,
    /// Version counter to detect invalidation.
    version: u64,
}

pub struct SyntaxHighlighter {
    parser: Parser,
    tree: Option<Tree>,
    language: EditorLanguage,
    highlight_query: Option<Query>,
    cache: Option<CachedHighlights>,
    version: u64,
}

impl SyntaxHighlighter {
    pub fn new(language: EditorLanguage) -> Self {
        let mut parser = Parser::new();
        let ts_lang = Self::get_language(language);
        let highlight_query = if let Some(ref lang) = ts_lang {
            if let Err(e) = parser.set_language(lang) {
                tracing::warn!("Failed to set parser language for {:?}: {}", language, e);
            }
            Self::get_highlight_query(language, lang)
        } else {
            None
        };

        Self {
            parser,
            tree: None,
            language,
            highlight_query,
            cache: None,
            version: 0,
        }
    }

    pub fn language(&self) -> EditorLanguage {
        self.language
    }

    /// Full parse of the entire buffer. Called on file open.
    pub fn parse_full(&mut self, rope: &Rope) {
        let source = rope.to_string();
        self.tree = self.parser.parse(&source, None);
        self.version += 1;
        self.cache = None;
    }

    /// Incremental parse after an edit.
    pub fn parse_incremental(
        &mut self,
        rope: &Rope,
        edits: &[super::buffer::InputEditInfo],
    ) {
        if let Some(ref mut tree) = self.tree {
            for edit in edits {
                tree.edit(&InputEdit {
                    start_byte: edit.start_byte,
                    old_end_byte: edit.old_end_byte,
                    new_end_byte: edit.new_end_byte,
                    start_position: Point::new(edit.start_row, edit.start_col),
                    old_end_position: Point::new(edit.old_end_row, edit.old_end_col),
                    new_end_position: Point::new(edit.new_end_row, edit.new_end_col),
                });
            }
        }
        let source = rope.to_string();
        self.tree = self.parser.parse(&source, self.tree.as_ref());
        self.version += 1;
        self.cache = None;
    }

    /// Get highlights for a range of lines. Returns a Vec of highlight spans per line.
    pub fn highlights_for_range(
        &mut self,
        rope: &Rope,
        start_line: usize,
        end_line: usize,
    ) -> Vec<Vec<HighlightSpan>> {
        // Check cache
        if let Some(ref cache) = self.cache {
            if cache.version == self.version
                && cache.start_line == start_line
                && cache.end_line == end_line
            {
                return cache.lines.clone();
            }
        }

        let result = self.compute_highlights(rope, start_line, end_line);

        self.cache = Some(CachedHighlights {
            start_line,
            end_line,
            lines: result.clone(),
            version: self.version,
        });

        result
    }

    fn compute_highlights(
        &self,
        rope: &Rope,
        start_line: usize,
        end_line: usize,
    ) -> Vec<Vec<HighlightSpan>> {
        let num_lines = end_line.saturating_sub(start_line);
        let mut result: Vec<Vec<HighlightSpan>> = vec![Vec::new(); num_lines];

        let tree = match &self.tree {
            Some(t) => t,
            None => return result,
        };
        let query = match &self.highlight_query {
            Some(q) => q,
            None => return result,
        };

        let source = rope.to_string();
        let source_bytes = source.as_bytes();

        // Restrict query to the visible byte range
        let start_byte = if start_line < rope.len_lines() {
            rope.line_to_byte(start_line)
        } else {
            rope.len_bytes()
        };
        let end_byte = if end_line < rope.len_lines() {
            rope.line_to_byte(end_line)
        } else {
            rope.len_bytes()
        };

        let mut cursor = QueryCursor::new();
        cursor.set_byte_range(start_byte..end_byte);

        let mut matches = cursor.matches(query, tree.root_node(), source_bytes);

        while let Some(m) = matches.next() {
            for capture in m.captures {
                let capture_name = &query.capture_names()[capture.index as usize];
                let (color, bold, italic) = highlight_name_to_style(capture_name);

                let node = capture.node;
                let node_start = node.start_byte();
                let node_end = node.end_byte();

                // Map byte range to lines
                let node_start_line = rope.byte_to_line(node_start.min(rope.len_bytes()));
                let node_end_line = rope.byte_to_line(node_end.min(rope.len_bytes()).max(1) - 1);

                for line_idx in node_start_line..=node_end_line {
                    if line_idx < start_line || line_idx >= end_line {
                        continue;
                    }
                    let line_byte_start = rope.line_to_byte(line_idx);
                    let line_byte_end = if line_idx + 1 < rope.len_lines() {
                        rope.line_to_byte(line_idx + 1)
                    } else {
                        rope.len_bytes()
                    };

                    let span_start = node_start.max(line_byte_start) - line_byte_start;
                    let span_end = node_end.min(line_byte_end) - line_byte_start;

                    if span_start < span_end {
                        let rel_line = line_idx - start_line;
                        result[rel_line].push(HighlightSpan {
                            range: span_start..span_end,
                            color,
                            bold,
                            italic,
                        });
                    }
                }
            }
        }

        // Sort spans per line by start position
        for spans in &mut result {
            spans.sort_by_key(|s| s.range.start);
        }

        result
    }

    fn get_language(lang: EditorLanguage) -> Option<tree_sitter::Language> {
        match lang {
            EditorLanguage::Rust => Some(tree_sitter_rust::LANGUAGE.into()),
            EditorLanguage::JavaScript | EditorLanguage::TypeScript => {
                Some(tree_sitter_javascript::LANGUAGE.into())
            }
            EditorLanguage::Python => Some(tree_sitter_python::LANGUAGE.into()),
            EditorLanguage::Bash => Some(tree_sitter_bash::LANGUAGE.into()),
            EditorLanguage::Json => Some(tree_sitter_json::LANGUAGE.into()),
            EditorLanguage::Toml => Some(tree_sitter_toml_ng::LANGUAGE.into()),
            EditorLanguage::Yaml => Some(tree_sitter_yaml::LANGUAGE.into()),
            EditorLanguage::Html => Some(tree_sitter_html::LANGUAGE.into()),
            EditorLanguage::Css => Some(tree_sitter_css::LANGUAGE.into()),
            EditorLanguage::Sql | EditorLanguage::PlainText => None,
        }
    }

    fn get_highlight_query(
        lang: EditorLanguage,
        ts_lang: &tree_sitter::Language,
    ) -> Option<Query> {
        let source = match lang {
            EditorLanguage::Rust => include_str!("queries/rust.scm"),
            EditorLanguage::JavaScript | EditorLanguage::TypeScript => {
                include_str!("queries/javascript.scm")
            }
            EditorLanguage::Python => include_str!("queries/python.scm"),
            EditorLanguage::Bash => include_str!("queries/bash.scm"),
            EditorLanguage::Json => include_str!("queries/json.scm"),
            EditorLanguage::Toml => include_str!("queries/toml.scm"),
            EditorLanguage::Yaml => include_str!("queries/yaml.scm"),
            EditorLanguage::Html => include_str!("queries/html.scm"),
            EditorLanguage::Css => include_str!("queries/css.scm"),
            EditorLanguage::Sql | EditorLanguage::PlainText => return None,
        };

        match Query::new(ts_lang, source) {
            Ok(q) => Some(q),
            Err(e) => {
                tracing::warn!("Failed to compile highlight query for {:?}: {}", lang, e);
                None
            }
        }
    }
}

/// Map tree-sitter capture names to editor colors.
fn highlight_name_to_style(name: &str) -> (Hsla, bool, bool) {
    // name may be dotted like "keyword.control" — match on the first segment
    let base = name.split('.').next().unwrap_or(name);

    match base {
        "keyword" | "conditional" | "repeat" | "include" | "exception" => {
            (ShellDeckColors::syntax_keyword(), true, false)
        }
        "function" | "method" => (ShellDeckColors::syntax_builtin(), false, false),
        "comment" => (ShellDeckColors::syntax_comment(), false, true),
        "string" | "character" => (ShellDeckColors::syntax_string(), false, false),
        "variable" | "parameter" | "field" | "property" => {
            (ShellDeckColors::syntax_variable(), false, false)
        }
        "operator" | "punctuation" => (ShellDeckColors::syntax_operator(), false, false),
        "number" | "float" => (ShellDeckColors::syntax_number(), false, false),
        "type" | "constructor" => (ShellDeckColors::syntax_builtin(), false, false),
        "constant" | "boolean" => (ShellDeckColors::syntax_number(), true, false),
        "attribute" | "tag" => (ShellDeckColors::syntax_keyword(), false, false),
        "label" | "namespace" => (ShellDeckColors::syntax_variable(), false, false),
        "escape" => (ShellDeckColors::syntax_command_sub(), false, false),
        _ => (ShellDeckColors::text_primary(), false, false),
    }
}
