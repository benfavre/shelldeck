use std::collections::HashSet;
use std::ops::Range;
use std::sync::OnceLock;

use super::bash::TokenKind;

#[derive(Debug, Clone)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

static PY_KEYWORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
static PY_BUILTINS: OnceLock<HashSet<&'static str>> = OnceLock::new();

fn keywords() -> &'static HashSet<&'static str> {
    PY_KEYWORDS.get_or_init(|| {
        [
            "False", "None", "True", "and", "as", "assert", "async", "await",
            "break", "class", "continue", "def", "del", "elif", "else", "except",
            "finally", "for", "from", "global", "if", "import", "in", "is",
            "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try",
            "while", "with", "yield",
        ]
        .into_iter()
        .collect()
    })
}

fn builtins() -> &'static HashSet<&'static str> {
    PY_BUILTINS.get_or_init(|| {
        [
            "print", "len", "range", "int", "str", "float", "list", "dict",
            "tuple", "set", "bool", "type", "isinstance", "issubclass", "hasattr",
            "getattr", "setattr", "delattr", "super", "property", "staticmethod",
            "classmethod", "enumerate", "zip", "map", "filter", "sorted", "reversed",
            "abs", "min", "max", "sum", "round", "input", "open", "repr",
            "format", "id", "hash", "dir", "vars", "globals", "locals",
            "callable", "iter", "next", "any", "all",
        ]
        .into_iter()
        .collect()
    })
}

/// Single-pass Python tokenizer.
pub fn tokenize(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        // Skip whitespace
        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
            i += 1;
            continue;
        }

        // Comment: # ...
        if b == b'#' {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            tokens.push(Token { range: start..i, kind: TokenKind::Comment });
            continue;
        }

        // Triple-quoted strings: """...""" or '''...'''
        if (b == b'"' || b == b'\'') && i + 2 < len && bytes[i + 1] == b && bytes[i + 2] == b {
            let start = i;
            let quote = b;
            i += 3;
            while i + 2 < len {
                if bytes[i] == quote && bytes[i + 1] == quote && bytes[i + 2] == quote {
                    i += 3;
                    break;
                }
                if bytes[i] == b'\\' { i += 1; }
                i += 1;
            }
            if i > len { i = len; }
            tokens.push(Token { range: start..i, kind: TokenKind::String });
            continue;
        }

        // Single/double-quoted strings
        if b == b'\'' || b == b'"' {
            let start = i;
            let quote = b;
            i += 1;
            while i < len && bytes[i] != quote {
                if bytes[i] == b'\\' && i + 1 < len { i += 1; }
                i += 1;
            }
            if i < len { i += 1; }
            tokens.push(Token { range: start..i, kind: TokenKind::String });
            continue;
        }

        // f-string prefix (f"..." or f'...')
        if (b == b'f' || b == b'F' || b == b'r' || b == b'R' || b == b'b' || b == b'B')
            && i + 1 < len && (bytes[i + 1] == b'"' || bytes[i + 1] == b'\'')
        {
            let start = i;
            i += 1;
            let quote = bytes[i];
            i += 1;
            while i < len && bytes[i] != quote {
                if bytes[i] == b'\\' && i + 1 < len { i += 1; }
                i += 1;
            }
            if i < len { i += 1; }
            tokens.push(Token { range: start..i, kind: TokenKind::String });
            continue;
        }

        // Numbers
        if b.is_ascii_digit() {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_') {
                i += 1;
            }
            tokens.push(Token { range: start..i, kind: TokenKind::Number });
            continue;
        }

        // Operators
        if matches!(b, b'+' | b'-' | b'*' | b'/' | b'%' | b'=' | b'<' | b'>' | b'!' | b'&' | b'|' | b'^' | b'~' | b'@') {
            let start = i;
            i += 1;
            // Consume double operators
            if i < len && matches!(bytes[i], b'=' | b'*' | b'/' | b'>' | b'<') {
                i += 1;
            }
            tokens.push(Token { range: start..i, kind: TokenKind::Operator });
            continue;
        }

        // Decorators: @...
        if b == b'@' {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.') {
                i += 1;
            }
            tokens.push(Token { range: start..i, kind: TokenKind::Builtin });
            continue;
        }

        // Words
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &source[start..i];

            let kind = if keywords().contains(word) {
                TokenKind::Keyword
            } else if builtins().contains(word) {
                TokenKind::Builtin
            } else if word.starts_with("__") && word.ends_with("__") {
                TokenKind::Variable // dunder methods
            } else {
                continue;
            };

            tokens.push(Token { range: start..i, kind });
            continue;
        }

        i += 1;
    }

    tokens
}
