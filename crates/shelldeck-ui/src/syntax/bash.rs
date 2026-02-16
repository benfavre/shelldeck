use std::collections::HashSet;
use std::ops::Range;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Keyword,
    Builtin,
    Comment,
    String,
    Variable,
    Operator,
    Number,
    Shebang,
    CommandSub,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

static KEYWORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
static BUILTINS: OnceLock<HashSet<&'static str>> = OnceLock::new();

fn keywords() -> &'static HashSet<&'static str> {
    KEYWORDS.get_or_init(|| {
        [
            "if", "then", "else", "elif", "fi", "for", "in", "while", "until", "do", "done",
            "case", "esac", "function", "return", "local", "export", "source", "readonly",
            "declare", "typeset", "unset", "select",
        ]
        .into_iter()
        .collect()
    })
}

fn builtins() -> &'static HashSet<&'static str> {
    BUILTINS.get_or_init(|| {
        [
            "echo", "cd", "pwd", "ls", "cat", "grep", "sed", "awk", "find", "sort", "head", "tail",
            "wc", "chmod", "chown", "mkdir", "rm", "cp", "mv", "test", "read", "printf", "exit",
            "true", "false", "shift", "set", "eval", "exec", "trap", "wait", "kill", "jobs", "bg",
            "fg", "alias", "unalias", "type", "which", "xargs", "tee", "curl", "wget",
        ]
        .into_iter()
        .collect()
    })
}

/// Single-pass bash tokenizer producing a list of tokens.
pub fn tokenize(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let len = bytes.len();
    let mut tokens = Vec::new();
    let mut i = 0;

    // Shebang: #! at byte 0
    if source.starts_with("#!") {
        let end = source.find('\n').unwrap_or(len);
        tokens.push(Token {
            range: 0..end,
            kind: TokenKind::Shebang,
        });
        i = end;
    }

    while i < len {
        let b = bytes[i];

        // Skip whitespace
        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
            i += 1;
            continue;
        }

        // Comment: # preceded by whitespace or at line start (not inside quotes)
        if b == b'#' {
            let at_line_start = i == 0 || bytes[i - 1] == b'\n';
            let after_whitespace = i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t');
            if at_line_start || after_whitespace {
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
        }

        // Single-quoted string
        if b == b'\'' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'\'' {
                i += 1;
            }
            if i < len {
                i += 1; // closing quote
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
                    i += 2; // skip escaped char
                    continue;
                }
                // Nested variable inside double quotes
                if bytes[i] == b'$' {
                    let var_start = i;
                    let var_end = scan_variable(bytes, i, len);
                    if var_end > i {
                        // Emit string up to variable if there's content
                        tokens.push(Token {
                            range: var_start..var_end,
                            kind: TokenKind::Variable,
                        });
                        i = var_end;
                        continue;
                    }
                }
                i += 1;
            }
            if i < len {
                i += 1; // closing quote
            }
            tokens.push(Token {
                range: start..i,
                kind: TokenKind::String,
            });
            continue;
        }

        // Variable: $...
        if b == b'$' {
            // Command substitution: $(...)
            if i + 1 < len && bytes[i + 1] == b'(' {
                let start = i;
                i += 2;
                let mut depth = 1;
                while i < len && depth > 0 {
                    if bytes[i] == b'(' {
                        depth += 1;
                    } else if bytes[i] == b')' {
                        depth -= 1;
                    }
                    i += 1;
                }
                tokens.push(Token {
                    range: start..i,
                    kind: TokenKind::CommandSub,
                });
                continue;
            }
            let start = i;
            let end = scan_variable(bytes, i, len);
            if end > i {
                tokens.push(Token {
                    range: start..end,
                    kind: TokenKind::Variable,
                });
                i = end;
                continue;
            }
            // Lone $ — skip
            i += 1;
            continue;
        }

        // Backtick command substitution
        if b == b'`' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'`' {
                if bytes[i] == b'\\' && i + 1 < len {
                    i += 2;
                    continue;
                }
                i += 1;
            }
            if i < len {
                i += 1;
            }
            tokens.push(Token {
                range: start..i,
                kind: TokenKind::CommandSub,
            });
            continue;
        }

        // Operators
        if let Some((op_len, _)) = match_operator(bytes, i, len) {
            tokens.push(Token {
                range: i..i + op_len,
                kind: TokenKind::Operator,
            });
            i += op_len;
            continue;
        }

        // Words (identifiers, keywords, builtins, numbers)
        if is_word_char(b) {
            let start = i;
            while i < len && is_word_char(bytes[i]) {
                i += 1;
            }
            let word = &source[start..i];

            let kind = if word.chars().all(|c| c.is_ascii_digit()) {
                TokenKind::Number
            } else if keywords().contains(word) {
                TokenKind::Keyword
            } else if builtins().contains(word) {
                TokenKind::Builtin
            } else {
                // Plain word — not a token we highlight
                continue;
            };

            tokens.push(Token {
                range: start..i,
                kind,
            });
            continue;
        }

        // Skip other characters
        i += 1;
    }

    tokens
}

/// Scan a variable starting at position `i` (which points to `$`).
/// Returns the end byte position.
fn scan_variable(bytes: &[u8], i: usize, len: usize) -> usize {
    if i + 1 >= len {
        return i;
    }
    let next = bytes[i + 1];

    // ${...}
    if next == b'{' {
        let mut j = i + 2;
        while j < len && bytes[j] != b'}' {
            j += 1;
        }
        if j < len {
            j += 1;
        }
        return j;
    }

    // Special variables: $@, $?, $$, $!, $#, $*, $0-$9
    if matches!(next, b'@' | b'?' | b'$' | b'!' | b'#' | b'*') || next.is_ascii_digit() {
        return i + 2;
    }

    // $WORD — alphanumeric + underscore
    if next == b'_' || next.is_ascii_alphabetic() {
        let mut j = i + 1;
        while j < len && (bytes[j] == b'_' || bytes[j].is_ascii_alphanumeric()) {
            j += 1;
        }
        return j;
    }

    i // No valid variable
}

fn match_operator(bytes: &[u8], i: usize, len: usize) -> Option<(usize, &'static str)> {
    if i + 1 < len {
        let two = &[bytes[i], bytes[i + 1]];
        match two {
            b"||" => return Some((2, "||")),
            b"&&" => return Some((2, "&&")),
            b";;" => return Some((2, ";;")),
            b">>" => return Some((2, ">>")),
            b"<<" => return Some((2, "<<")),
            b"2>" => return Some((2, "2>")),
            b"&>" => return Some((2, "&>")),
            _ => {}
        }
    }
    match bytes[i] {
        b'|' => Some((1, "|")),
        b';' => Some((1, ";")),
        b'>' => Some((1, ">")),
        b'<' => Some((1, "<")),
        b'!' => Some((1, "!")),
        _ => None,
    }
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}
