use crate::grid::{Cell, CellWidth};
use regex::Regex;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlKind {
    Web,
    FilePath {
        line: Option<u32>,
        col: Option<u32>,
    },
}

#[derive(Debug, Clone)]
pub struct UrlMatch {
    pub row: usize,
    pub col: usize,
    pub len: usize,
    pub url: String,
    pub kind: UrlKind,
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

/// Detect URLs in the given visible rows.
pub fn detect_urls(visible_rows: &[&Vec<Cell>]) -> Vec<UrlMatch> {
    let re = url_regex();
    let mut matches = Vec::new();

    for (ri, row) in visible_rows.iter().enumerate() {
        let line: String = row.iter()
            .filter(|c| c.wide != CellWidth::Spacer)
            .map(|c| c.c)
            .collect();
        for m in re.find_iter(&line) {
            let raw = m.as_str();
            let trimmed = trim_trailing(raw);
            if trimmed.is_empty() {
                continue;
            }

            let kind = if trimmed.starts_with('/') || trimmed.starts_with("~/") || trimmed.starts_with("./") || trimmed.starts_with("../") {
                // Try to extract line:col from patterns like file.rs:42:10
                let (_, line_num, col_num) = parse_file_location(trimmed);
                UrlKind::FilePath {
                    line: line_num,
                    col: col_num,
                }
            } else {
                UrlKind::Web
            };

            matches.push(UrlMatch {
                row: ri,
                col: m.start(),
                len: trimmed.len(),
                url: trimmed.to_string(),
                kind,
            });
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
                let path_end = s.rfind(':').expect("split(':') produced 3 parts so ':' exists");
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
