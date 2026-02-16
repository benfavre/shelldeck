use std::collections::HashSet;
use std::ops::Range;
use std::sync::OnceLock;

use super::bash::TokenKind;

#[derive(Debug, Clone)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

static PHP_KEYWORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
static PHP_BUILTINS: OnceLock<HashSet<&'static str>> = OnceLock::new();

fn keywords() -> &'static HashSet<&'static str> {
    PHP_KEYWORDS.get_or_init(|| {
        [
            "abstract",
            "and",
            "array",
            "as",
            "break",
            "callable",
            "case",
            "catch",
            "class",
            "clone",
            "const",
            "continue",
            "declare",
            "default",
            "do",
            "echo",
            "else",
            "elseif",
            "empty",
            "enddeclare",
            "endfor",
            "endforeach",
            "endif",
            "endswitch",
            "endwhile",
            "eval",
            "exit",
            "extends",
            "final",
            "finally",
            "fn",
            "for",
            "foreach",
            "function",
            "global",
            "goto",
            "if",
            "implements",
            "include",
            "include_once",
            "instanceof",
            "insteadof",
            "interface",
            "isset",
            "list",
            "match",
            "namespace",
            "new",
            "or",
            "print",
            "private",
            "protected",
            "public",
            "readonly",
            "require",
            "require_once",
            "return",
            "static",
            "switch",
            "throw",
            "trait",
            "try",
            "unset",
            "use",
            "var",
            "while",
            "xor",
            "yield",
            "true",
            "false",
            "null",
            "self",
            "parent",
        ]
        .into_iter()
        .collect()
    })
}

fn builtins() -> &'static HashSet<&'static str> {
    PHP_BUILTINS.get_or_init(|| {
        [
            "php_uname",
            "phpversion",
            "phpinfo",
            "strlen",
            "strpos",
            "substr",
            "explode",
            "implode",
            "str_replace",
            "trim",
            "strtolower",
            "strtoupper",
            "array_map",
            "array_filter",
            "array_merge",
            "array_push",
            "array_pop",
            "array_keys",
            "array_values",
            "count",
            "in_array",
            "sort",
            "rsort",
            "json_encode",
            "json_decode",
            "file_get_contents",
            "file_put_contents",
            "var_dump",
            "print_r",
            "get_loaded_extensions",
            "PHP_VERSION",
        ]
        .into_iter()
        .collect()
    })
}

/// Single-pass PHP tokenizer.
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

        // PHP tags
        if b == b'<' && i + 4 < len && &source[i..i + 5] == "<?php" {
            tokens.push(Token {
                range: i..i + 5,
                kind: TokenKind::Keyword,
            });
            i += 5;
            continue;
        }
        if b == b'?' && i + 1 < len && bytes[i + 1] == b'>' {
            tokens.push(Token {
                range: i..i + 2,
                kind: TokenKind::Keyword,
            });
            i += 2;
            continue;
        }

        // Line comment: // ... or # ...
        if (b == b'/' && i + 1 < len && bytes[i + 1] == b'/') || b == b'#' {
            let start = i;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            tokens.push(Token {
                range: start..i,
                kind: TokenKind::Comment,
            });
            continue;
        }

        // Block comment: /* ... */
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < len {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }
            if i > len {
                i = len;
            }
            tokens.push(Token {
                range: start..i,
                kind: TokenKind::Comment,
            });
            continue;
        }

        // Single-quoted string
        if b == b'\'' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'\'' {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 1;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            tokens.push(Token {
                range: start..i,
                kind: TokenKind::String,
            });
            continue;
        }

        // Double-quoted string
        if b == b'"' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 1;
                }
                // $variable inside string
                if bytes[i] == b'$' {
                    let var_start = i;
                    i += 1;
                    while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                        i += 1;
                    }
                    if i > var_start + 1 {
                        tokens.push(Token {
                            range: var_start..i,
                            kind: TokenKind::Variable,
                        });
                    }
                    continue;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            tokens.push(Token {
                range: start..i,
                kind: TokenKind::String,
            });
            continue;
        }

        // Variable: $...
        if b == b'$' {
            let start = i;
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            if i > start + 1 {
                tokens.push(Token {
                    range: start..i,
                    kind: TokenKind::Variable,
                });
            }
            continue;
        }

        // Numbers
        if b.is_ascii_digit() {
            let start = i;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'.' || bytes[i] == b'_')
            {
                i += 1;
            }
            tokens.push(Token {
                range: start..i,
                kind: TokenKind::Number,
            });
            continue;
        }

        // Operators
        if matches!(
            b,
            b'+' | b'-'
                | b'*'
                | b'/'
                | b'%'
                | b'='
                | b'<'
                | b'>'
                | b'!'
                | b'&'
                | b'|'
                | b'^'
                | b'~'
                | b'.'
                | b'?'
                | b':'
        ) {
            let start = i;
            i += 1;
            if i < len && matches!(bytes[i], b'=' | b'>' | b'.' | b'&' | b'|') {
                i += 1;
            }
            tokens.push(Token {
                range: start..i,
                kind: TokenKind::Operator,
            });
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
            } else {
                continue;
            };

            tokens.push(Token {
                range: start..i,
                kind,
            });
            continue;
        }

        i += 1;
    }

    tokens
}
