use crate::grid::{Cell, CellWidth, MouseMode, TerminalGrid};
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlKind {
    Web,
    FilePath { line: Option<u32>, col: Option<u32> },
}

#[derive(Debug, Clone)]
pub struct UrlMatch {
    pub row: usize,
    pub col: usize,
    pub len: usize,
    pub url: String,
    pub kind: UrlKind,
}

impl UrlMatch {
    pub fn contains(&self, col: usize, row: usize) -> bool {
        self.row == row && col >= self.col && col - self.col < self.len
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum LeftClickAction {
    OpenUrl,
    ReportMouse,
    Select,
}

pub fn left_click_action(
    mouse_mode: MouseMode,
    link_modifier: bool,
    has_url: bool,
) -> LeftClickAction {
    if link_modifier && has_url {
        LeftClickAction::OpenUrl
    } else if mouse_mode != MouseMode::None {
        LeftClickAction::ReportMouse
    } else {
        LeftClickAction::Select
    }
}

static URL_REGEX: OnceLock<Regex> = OnceLock::new();

fn url_regex() -> &'static Regex {
    URL_REGEX.get_or_init(|| {
        Regex::new(concat!(
            r"(?:",
            // Web URLs
            r"(?:https?|ftp|file|ssh|git)://[^\s<>\[\]{}|\\^`\x00-\x1f]+",
            r"|",
            // mailto: links
            r"mailto:[^\s<>\[\]{}|\\^`\x00-\x1f]+",
            r"|",
            // File paths: /absolute, ~/home, ./relative, ../parent
            r"(?:/|~/|\.\.?/)[^\s<>\[\]{}|\\^`\x00-\x1f]+",
            r")",
        ))
        .expect("URL regex compilation failed")
    })
}

/// Trim trailing punctuation that likely isn't part of the URL.
/// Handles balanced parentheses (common in Wikipedia URLs).
fn trim_trailing(url: &str) -> &str {
    let mut s = url;
    loop {
        let prev = s;
        // Trim trailing punctuation that's usually not part of URLs
        s = s.trim_end_matches(['.', ',', ';', ':', '!', '?']);

        // Balance parentheses: if closing > opening, trim trailing )
        let open = s.matches('(').count();
        let close = s.matches(')').count();
        if close > open && s.ends_with(')') {
            s = &s[..s.len() - 1];
        }

        // Balance brackets similarly
        let open_b = s.matches('[').count();
        let close_b = s.matches(']').count();
        if close_b > open_b && s.ends_with(']') {
            s = &s[..s.len() - 1];
        }

        // Trim trailing quotes
        s = s.trim_end_matches(['\'', '"']);

        if s == prev {
            break;
        }
    }
    s
}

/// Detect URLs using the grid's wrap metadata, including the scrollback viewport.
pub fn detect_grid_urls(grid: &TerminalGrid) -> Vec<UrlMatch> {
    let rows = grid.visible_rows();
    let wraps: Vec<bool> = (0..rows.len())
        .map(|row| grid.visible_row_is_soft_wrapped(row))
        .collect();
    detect_urls_with_wraps(&rows, &wraps)
}

/// Detect URLs in rows without soft-wrap metadata.
pub fn detect_urls(visible_rows: &[&Vec<Cell>]) -> Vec<UrlMatch> {
    detect_urls_with_wraps(visible_rows, &[])
}

fn detect_urls_with_wraps(visible_rows: &[&Vec<Cell>], wraps: &[bool]) -> Vec<UrlMatch> {
    let re = url_regex();
    let mut text = String::new();
    // Regex offsets are UTF-8 bytes; rendering and pointer hits use terminal cells.
    let mut positions = Vec::new();

    for (ri, row) in visible_rows.iter().enumerate() {
        for (col, cell) in row.iter().enumerate() {
            if cell.wide == CellWidth::Spacer {
                continue;
            }
            let start = text.len();
            text.push(cell.c);
            text.extend(cell.combining.iter());
            let width = if cell.wide == CellWidth::Wide { 2 } else { 1 };
            positions.push((start, text.len(), ri, col, width));
        }

        let soft_wrap = wraps.get(ri + 1).copied().unwrap_or(false);
        // Full-screen CLIs (including Claude's login screen) explicitly redraw
        // wrapped query URLs with CR/LF. tmux cannot preserve a soft-wrap flag
        // for them. Only join a query URL that reaches the terminal's right edge
        // with an unindented URL token; stop at blanks, prose, or another URL.
        let query_continuation = visible_rows.get(ri + 1).is_some_and(|next| {
            let next_text: String = next
                .iter()
                .filter(|c| c.wide != CellWidth::Spacer)
                .map(|c| c.c)
                .collect();
            let token = next_text.trim_end();
            !token.is_empty()
                && !token
                    .chars()
                    .any(|c| c.is_whitespace() || "<>[]{}|\\^`".contains(c))
                && !token.contains("://")
                && row.last().is_some_and(|c| !c.c.is_whitespace())
                && re
                    .find_iter(text.rsplit('\n').next().unwrap_or(""))
                    .last()
                    .is_some_and(|m| {
                        m.end() == text.rsplit('\n').next().unwrap_or("").len()
                            && (m.as_str().starts_with("https://")
                                || m.as_str().starts_with("http://"))
                            && m.as_str().contains('?')
                    })
        });
        if !soft_wrap && !query_continuation {
            text.push('\n');
        }
    }

    let mut matches: Vec<UrlMatch> = Vec::new();
    for m in re.find_iter(&text) {
        let trimmed = trim_trailing(m.as_str());
        if trimmed.is_empty() {
            continue;
        }
        let kind = if trimmed.starts_with('/')
            || trimmed.starts_with("~/")
            || trimmed.starts_with("./")
            || trimmed.starts_with("../")
        {
            let (_, line, col) = parse_file_location(trimmed);
            UrlKind::FilePath { line, col }
        } else {
            UrlKind::Web
        };
        let end = m.start() + trimmed.len();
        let start_pos = positions.partition_point(|&(_, end, _, _, _)| end <= m.start());
        let mut segment: Option<UrlMatch> = None;
        for &(start, _, row, col, width) in &positions[start_pos..] {
            if start >= end {
                break;
            }
            if let Some(current) = segment.as_mut().filter(|s| s.row == row) {
                current.len = col + width - current.col;
            } else {
                if let Some(previous) = segment.take() {
                    matches.push(previous);
                }
                segment = Some(UrlMatch {
                    row,
                    col,
                    len: width,
                    url: trimmed.to_string(),
                    kind: kind.clone(),
                });
            }
        }
        if let Some(last) = segment {
            matches.push(last);
        }
    }
    matches
}

/// Parse a file path that may have :line:col suffix.
/// Returns (path, Option<line>, Option<col>).
fn parse_file_location(s: &str) -> (&str, Option<u32>, Option<u32>) {
    // Match patterns like path:line:col or path:line
    let parts: Vec<&str> = s.rsplitn(3, ':').collect();
    match parts.len() {
        3 => {
            if let (Ok(line), Ok(col)) = (parts[1].parse::<u32>(), parts[0].parse::<u32>()) {
                (parts[2], Some(line), Some(col))
            } else if let Ok(line) = parts[0].parse::<u32>() {
                // Last segment was the line
                let path_end = s
                    .rfind(':')
                    .expect("split(':') produced 3 parts so ':' exists");
                (&s[..path_end], Some(line), None)
            } else {
                (s, None, None)
            }
        }
        2 => {
            if let Ok(line) = parts[0].parse::<u32>() {
                (parts[1], Some(line), None)
            } else {
                (s, None, None)
            }
        }
        _ => (s, None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_from_text(text: &str) -> Vec<Cell> {
        text.chars()
            .map(|c| Cell {
                c,
                ..Cell::default()
            })
            .collect()
    }

    #[test]
    fn trims_trailing_punctuation_from_detected_web_url() {
        let row = row_from_text("See https://example.com/docs, for details.");
        let urls = detect_urls(&[&row]);

        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].url, "https://example.com/docs");
        assert_eq!(urls[0].kind, UrlKind::Web);
    }

    #[test]
    fn detects_file_path_with_line_and_column() {
        let row = row_from_text("Error at ./src/main.rs:42:7");
        let urls = detect_urls(&[&row]);

        assert_eq!(urls.len(), 1);
        assert_eq!(urls[0].url, "./src/main.rs:42:7");
        assert_eq!(
            urls[0].kind,
            UrlKind::FilePath {
                line: Some(42),
                col: Some(7),
            }
        );
    }

    #[test]
    fn parses_path_that_contains_colons_and_line_suffix() {
        let (path, line, col) = parse_file_location("/tmp/svc:api/main.rs:120");

        assert_eq!(path, "/tmp/svc:api/main.rs");
        assert_eq!(line, Some(120));
        assert_eq!(col, None);
    }

    // SDTEST-1945 — SDUC-021: replay a CLI redraw, not a terminal soft wrap.
    #[test]
    fn hard_wrapped_login_query_opens_the_complete_url_from_every_row() {
        let url = format!(
            "https://example.com/oauth/authorize?client_id={}&response_type=code&code_challenge={}",
            "a".repeat(100),
            "b".repeat(180)
        );
        let width = 79;
        let rows: Vec<Vec<Cell>> = url
            .as_bytes()
            .chunks(width)
            .map(|chunk| {
                let mut row = row_from_text(std::str::from_utf8(chunk).unwrap());
                row.resize(width, Cell::default());
                row
            })
            .collect();
        let visible: Vec<_> = rows.iter().collect();
        let matches = detect_urls(&visible);
        assert_eq!(matches.len(), rows.len());
        for (row, segment) in matches.iter().enumerate() {
            assert_eq!(segment.url, url);
            assert!(segment.contains(0, row));
            assert!(segment.contains(segment.len - 1, row));
            assert!(!segment.contains(segment.len, row));
        }
    }

    // SDTEST-1946 — SDUC-021: keep hard newlines and unrelated links separate.
    #[test]
    fn query_wrap_recovery_stops_at_whitespace_blank_rows_and_another_url() {
        let first = row_from_text("https://example.com/?q=one");
        for next in [
            "next command output",
            " continuation",
            "",
            "https://other.example/?q=two",
        ] {
            let next = row_from_text(next);
            let matches = detect_urls(&[&first, &next]);
            assert_eq!(matches[0].url, "https://example.com/?q=one");
        }
        let mut short = first.clone();
        short.resize(short.len() + 1, Cell::default());
        let next = row_from_text("continuation");
        assert_eq!(
            detect_urls(&[&short, &next])[0].url,
            "https://example.com/?q=one"
        );
    }

    // SDTEST-1947 — SDUC-021: pointer cells must not be UTF-8 byte offsets.
    #[test]
    fn unicode_and_wide_cells_keep_link_hitboxes_aligned() {
        let mut row = row_from_text("é ");
        row.push(Cell {
            c: '界',
            wide: CellWidth::Wide,
            ..Cell::default()
        });
        row.push(Cell {
            wide: CellWidth::Spacer,
            ..Cell::default()
        });
        row.extend(row_from_text(" https://example.com/été"));
        let found = detect_urls(&[&row]);
        let link = &found[0];
        assert_eq!(link.col, 5);
        assert_eq!(link.len, "https://example.com/été".chars().count());
        assert!(link.contains(5, 0));
        assert!(!link.contains(4, 0));
        assert!(!link.contains(5 + link.len, 0));
    }

    // SDTEST-1948 — SDUC-021: real parser/scrollback wrap metadata is retained.
    #[test]
    fn soft_wrapped_urls_use_the_scrolled_viewport_coordinates() {
        use crate::parser::TerminalProcessor;
        use parking_lot::Mutex;
        use std::sync::Arc;
        let url = "https://example.com/a-long-path-without-a-query";
        let grid = Arc::new(Mutex::new(TerminalGrid::new(3, 20)));
        let mut processor = TerminalProcessor::new(grid.clone());
        processor.process_bytes(
            &mut vte::Parser::new(),
            format!("{url}\r\nnext\r\n").as_bytes(),
        );
        let mut grid = grid.lock();
        grid.scroll_view_up(2);
        let matches = detect_grid_urls(&grid);
        assert_eq!(matches.len(), 3);
        assert!(matches.iter().all(|segment| segment.url == url));
        assert!(matches[0].contains(0, 0));
        assert!(matches[2].contains(0, 2));
    }
    // SDTEST-1949 — SDUC-021: tmux cannot swallow Ctrl/Cmd-click on a link.
    #[test]
    fn link_activation_precedes_mouse_reporting_without_stealing_ordinary_clicks() {
        for mode in [
            MouseMode::None,
            MouseMode::Press,
            MouseMode::ButtonTracking,
            MouseMode::AnyMotion,
        ] {
            assert_eq!(
                left_click_action(mode, true, true),
                LeftClickAction::OpenUrl
            );
            let ordinary = if mode == MouseMode::None {
                LeftClickAction::Select
            } else {
                LeftClickAction::ReportMouse
            };
            assert_eq!(left_click_action(mode, false, true), ordinary);
            assert_eq!(left_click_action(mode, true, false), ordinary);
        }
    }
}
