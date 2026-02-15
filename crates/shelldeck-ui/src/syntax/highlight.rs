use std::ops::Range;

use gpui::*;

use shelldeck_core::models::script::ScriptLanguage;

use crate::syntax::bash::{self, TokenKind};
use crate::syntax::{javascript, php, python, sql};
use crate::theme::ShellDeckColors;

fn token_style(kind: TokenKind) -> HighlightStyle {
    match kind {
        TokenKind::Keyword => HighlightStyle {
            color: Some(ShellDeckColors::syntax_keyword()),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        },
        TokenKind::Builtin => HighlightStyle {
            color: Some(ShellDeckColors::syntax_builtin()),
            ..Default::default()
        },
        TokenKind::Comment => HighlightStyle {
            color: Some(ShellDeckColors::syntax_comment()),
            font_style: Some(FontStyle::Italic),
            ..Default::default()
        },
        TokenKind::String => HighlightStyle {
            color: Some(ShellDeckColors::syntax_string()),
            ..Default::default()
        },
        TokenKind::Variable => HighlightStyle {
            color: Some(ShellDeckColors::syntax_variable()),
            ..Default::default()
        },
        TokenKind::Operator => HighlightStyle {
            color: Some(ShellDeckColors::syntax_operator()),
            ..Default::default()
        },
        TokenKind::Number => HighlightStyle {
            color: Some(ShellDeckColors::syntax_number()),
            ..Default::default()
        },
        TokenKind::Shebang => HighlightStyle {
            color: Some(ShellDeckColors::syntax_comment()),
            font_style: Some(FontStyle::Italic),
            ..Default::default()
        },
        TokenKind::CommandSub => HighlightStyle {
            color: Some(ShellDeckColors::syntax_command_sub()),
            ..Default::default()
        },
    }
}

/// Convert bash source into GPUI highlight ranges.
pub fn highlight_bash(source: &str) -> Vec<(Range<usize>, HighlightStyle)> {
    bash::tokenize(source)
        .into_iter()
        .map(|tok| (tok.range, token_style(tok.kind)))
        .collect()
}

/// Highlight source code for a specific language.
pub fn highlight_for_language(
    source: &str,
    language: &ScriptLanguage,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let mut highlights = match language {
        ScriptLanguage::Shell => highlight_bash(source),
        ScriptLanguage::Python => python::tokenize(source)
            .into_iter()
            .map(|tok| (tok.range, token_style(tok.kind)))
            .collect(),
        ScriptLanguage::Node | ScriptLanguage::Bun => javascript::tokenize(source)
            .into_iter()
            .map(|tok| (tok.range, token_style(tok.kind)))
            .collect(),
        ScriptLanguage::Mysql | ScriptLanguage::Postgresql => sql::tokenize(source)
            .into_iter()
            .map(|tok| (tok.range, token_style(tok.kind)))
            .collect(),
        ScriptLanguage::Php => php::tokenize(source)
            .into_iter()
            .map(|tok| (tok.range, token_style(tok.kind)))
            .collect(),
        // Docker, Systemd, Nginx, Custom — use bash highlighting as fallback
        _ => highlight_bash(source),
    };

    // Post-process: highlight {{variable}} patterns across all languages
    highlight_template_variables(source, &mut highlights);

    highlights
}

/// Scan for `{{...}}` template variable patterns and add highlights.
/// Overwrites any existing highlights in those ranges.
fn highlight_template_variables(
    source: &str,
    highlights: &mut Vec<(Range<usize>, HighlightStyle)>,
) {
    let var_style = HighlightStyle {
        color: Some(ShellDeckColors::syntax_template_var()),
        font_style: Some(FontStyle::Italic),
        font_weight: Some(FontWeight::MEDIUM),
        ..Default::default()
    };

    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i + 1 < len {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let start = i;
            let mut j = i + 2;
            while j + 1 < len {
                if bytes[j] == b'}' && bytes[j + 1] == b'}' {
                    let end = j + 2;
                    // Remove any existing highlights that overlap this range
                    highlights.retain(|h| h.0.end <= start || h.0.start >= end);
                    highlights.push((start..end, var_style));
                    i = end;
                    break;
                }
                j += 1;
            }
            if j + 1 >= len {
                break;
            }
        } else {
            i += 1;
        }
    }
}

/// Render a code block with line numbers and syntax highlighting.
///
/// - `body`: the source text
/// - `cursor`: optional `(line, col)` for cursor display (0-indexed)
/// - `is_active`: whether the editor is focused (affects cursor visibility)
pub fn render_code_block(
    body: &str,
    cursor: Option<(usize, usize)>,
    is_active: bool,
) -> Div {
    render_code_block_with_language(body, cursor, is_active, &ScriptLanguage::Shell)
}

/// Render a code block with line numbers, syntax highlighting for a specific language.
pub fn render_code_block_with_language(
    body: &str,
    cursor: Option<(usize, usize)>,
    is_active: bool,
    language: &ScriptLanguage,
) -> Div {
    let highlights = highlight_for_language(body, language);
    let lines: Vec<&str> = body.split('\n').collect();
    let line_count = lines.len();
    let gutter_width = format!("{}", line_count).len().max(2) as f32 * 8.0 + 16.0;

    let mut rows = div().flex().flex_col().w_full();

    let mut byte_offset = 0usize;

    for (line_idx, line_text) in lines.iter().enumerate() {
        let line_start = byte_offset;
        let line_end = line_start + line_text.len();

        // Line number
        let line_num = div()
            .w(px(gutter_width))
            .flex_shrink_0()
            .text_size(px(13.0))
            .text_color(ShellDeckColors::line_number_fg())
            .bg(ShellDeckColors::line_number_bg())
            .pr(px(8.0))
            .pl(px(4.0))
            .py(px(1.0))
            .flex()
            .justify_end()
            .child(format!("{}", line_idx + 1));

        // Compute line-relative highlights
        let mut line_highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
        for (range, style) in &highlights {
            if range.end <= line_start || range.start >= line_end {
                continue;
            }
            let rel_start = range.start.max(line_start) - line_start;
            let rel_end = range.end.min(line_end) - line_start;
            if rel_start < rel_end {
                line_highlights.push((rel_start..rel_end, *style));
            }
        }

        // Use a display string — empty lines need at least a space for height
        let display_text: SharedString = if line_text.is_empty() {
            " ".into()
        } else {
            line_text.to_string().into()
        };

        let styled = StyledText::new(display_text).with_highlights(line_highlights);

        let is_cursor_line = cursor.map(|(l, _)| l == line_idx).unwrap_or(false);

        let mut code_cell = div()
            .flex_grow()
            .min_w_0()
            .overflow_hidden()
            .pl(px(8.0))
            .py(px(1.0))
            .text_size(px(13.0))
            .text_color(ShellDeckColors::text_primary())
            .relative();

        if is_cursor_line {
            code_cell = code_cell.bg(ShellDeckColors::cursor_line_bg());
        }

        code_cell = code_cell.child(styled);

        // Cursor indicator
        if is_active && is_cursor_line {
            if let Some((_, col)) = cursor {
                let cursor_x = col as f32 * 7.8;
                code_cell = code_cell.child(
                    div()
                        .absolute()
                        .top(px(1.0))
                        .left(px(8.0 + cursor_x))
                        .w(px(1.5))
                        .h(px(16.0))
                        .bg(ShellDeckColors::primary()),
                );
            }
        }

        let row = div().flex().w_full().child(line_num).child(code_cell);
        rows = rows.child(row);

        byte_offset = line_end + 1; // +1 for '\n'
    }

    rows
}
