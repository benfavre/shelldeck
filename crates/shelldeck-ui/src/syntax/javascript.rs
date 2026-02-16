use std::collections::HashSet;
use std::ops::Range;
use std::sync::OnceLock;

use super::bash::TokenKind;

#[derive(Debug, Clone)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

static JS_KEYWORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
static JS_BUILTINS: OnceLock<HashSet<&'static str>> = OnceLock::new();

fn keywords() -> &'static HashSet<&'static str> {
    JS_KEYWORDS.get_or_init(|| {
        [
            "async",
            "await",
            "break",
            "case",
            "catch",
            "class",
            "const",
            "continue",
            "debugger",
            "default",
            "delete",
            "do",
            "else",
            "export",
            "extends",
            "finally",
            "for",
            "from",
            "function",
            "if",
            "import",
            "in",
            "instanceof",
            "let",
            "new",
            "of",
            "return",
            "static",
            "super",
            "switch",
            "this",
            "throw",
            "try",
            "typeof",
            "var",
            "void",
            "while",
            "with",
            "yield",
            "enum",
            "implements",
            "interface",
            "package",
            "private",
            "protected",
            "public",
            "abstract",
            "type",
            "declare",
            "namespace",
            "module",
            // Bun/TS specifics
            "as",
            "satisfies",
            "readonly",
            "keyof",
            "infer",
        ]
        .into_iter()
        .collect()
    })
}

fn builtins() -> &'static HashSet<&'static str> {
    JS_BUILTINS.get_or_init(|| {
        [
            "console",
            "require",
            "module",
            "exports",
            "process",
            "Buffer",
            "setTimeout",
            "setInterval",
            "clearTimeout",
            "clearInterval",
            "Promise",
            "JSON",
            "Math",
            "Date",
            "Array",
            "Object",
            "String",
            "Number",
            "Boolean",
            "Map",
            "Set",
            "WeakMap",
            "WeakSet",
            "Symbol",
            "Proxy",
            "Reflect",
            "Error",
            "RegExp",
            "parseInt",
            "parseFloat",
            "isNaN",
            "isFinite",
            "undefined",
            "null",
            "true",
            "false",
            "NaN",
            "Infinity",
            "globalThis",
            "fetch",
            "Response",
            "Request",
            "URL",
            // Bun specifics
            "Bun",
        ]
        .into_iter()
        .collect()
    })
}

/// Single-pass JavaScript/TypeScript tokenizer.
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

        // Line comment: // ...
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
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

        // Template literals: `...`
        if b == b'`' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'`' {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 1;
                }
                // ${...} interpolation
                if bytes[i] == b'$' && i + 1 < len && bytes[i + 1] == b'{' {
                    let var_start = i;
                    i += 2;
                    let mut depth = 1;
                    while i < len && depth > 0 {
                        if bytes[i] == b'{' {
                            depth += 1;
                        } else if bytes[i] == b'}' {
                            depth -= 1;
                        }
                        i += 1;
                    }
                    tokens.push(Token {
                        range: var_start..i,
                        kind: TokenKind::Variable,
                    });
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

        // Single/double-quoted strings
        if b == b'\'' || b == b'"' {
            let start = i;
            let quote = b;
            i += 1;
            while i < len && bytes[i] != quote {
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

        // Numbers
        if b.is_ascii_digit() || (b == b'.' && i + 1 < len && bytes[i + 1].is_ascii_digit()) {
            let start = i;
            // Handle 0x, 0b, 0o prefixes
            if b == b'0'
                && i + 1 < len
                && matches!(bytes[i + 1], b'x' | b'X' | b'b' | b'B' | b'o' | b'O')
            {
                i += 2;
                while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
            } else {
                while i < len
                    && (bytes[i].is_ascii_digit()
                        || bytes[i] == b'.'
                        || bytes[i] == b'_'
                        || bytes[i] == b'e'
                        || bytes[i] == b'E')
                {
                    i += 1;
                }
            }
            // Optional n suffix for BigInt
            if i < len && bytes[i] == b'n' {
                i += 1;
            }
            tokens.push(Token {
                range: start..i,
                kind: TokenKind::Number,
            });
            continue;
        }

        // Arrow operator
        if b == b'=' && i + 1 < len && bytes[i + 1] == b'>' {
            tokens.push(Token {
                range: i..i + 2,
                kind: TokenKind::Operator,
            });
            i += 2;
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
                | b'?'
        ) {
            let start = i;
            i += 1;
            while i < len && matches!(bytes[i], b'=' | b'>' | b'<' | b'&' | b'|' | b'?' | b'.') {
                i += 1;
            }
            tokens.push(Token {
                range: start..i,
                kind: TokenKind::Operator,
            });
            continue;
        }

        // Words
        if b.is_ascii_alphabetic() || b == b'_' || b == b'$' {
            let start = i;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'$')
            {
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
