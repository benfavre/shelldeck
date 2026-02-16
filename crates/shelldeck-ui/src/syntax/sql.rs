use std::collections::HashSet;
use std::ops::Range;
use std::sync::OnceLock;

use super::bash::TokenKind;

#[derive(Debug, Clone)]
pub struct Token {
    pub range: Range<usize>,
    pub kind: TokenKind,
}

static SQL_KEYWORDS: OnceLock<HashSet<&'static str>> = OnceLock::new();
static SQL_FUNCTIONS: OnceLock<HashSet<&'static str>> = OnceLock::new();

fn keywords() -> &'static HashSet<&'static str> {
    SQL_KEYWORDS.get_or_init(|| {
        [
            "SELECT",
            "FROM",
            "WHERE",
            "INSERT",
            "INTO",
            "UPDATE",
            "DELETE",
            "SET",
            "CREATE",
            "DROP",
            "ALTER",
            "TABLE",
            "INDEX",
            "VIEW",
            "DATABASE",
            "SCHEMA",
            "JOIN",
            "INNER",
            "LEFT",
            "RIGHT",
            "OUTER",
            "CROSS",
            "ON",
            "AS",
            "AND",
            "OR",
            "NOT",
            "IN",
            "IS",
            "NULL",
            "LIKE",
            "BETWEEN",
            "EXISTS",
            "HAVING",
            "GROUP",
            "BY",
            "ORDER",
            "ASC",
            "DESC",
            "LIMIT",
            "OFFSET",
            "UNION",
            "ALL",
            "DISTINCT",
            "VALUES",
            "DEFAULT",
            "PRIMARY",
            "KEY",
            "FOREIGN",
            "REFERENCES",
            "CONSTRAINT",
            "CHECK",
            "UNIQUE",
            "IF",
            "THEN",
            "ELSE",
            "WHEN",
            "CASE",
            "END",
            "BEGIN",
            "COMMIT",
            "ROLLBACK",
            "TRANSACTION",
            "GRANT",
            "REVOKE",
            "WITH",
            "RECURSIVE",
            "EXPLAIN",
            "SHOW",
            "DESCRIBE",
            "USE",
            "TRUNCATE",
            // lowercase aliases
            "select",
            "from",
            "where",
            "insert",
            "into",
            "update",
            "delete",
            "set",
            "create",
            "drop",
            "alter",
            "table",
            "index",
            "view",
            "database",
            "schema",
            "join",
            "inner",
            "left",
            "right",
            "outer",
            "cross",
            "on",
            "as",
            "and",
            "or",
            "not",
            "in",
            "is",
            "null",
            "like",
            "between",
            "exists",
            "having",
            "group",
            "by",
            "order",
            "asc",
            "desc",
            "limit",
            "offset",
            "union",
            "all",
            "distinct",
            "values",
            "default",
            "primary",
            "key",
            "foreign",
            "references",
            "constraint",
            "check",
            "unique",
            "if",
            "then",
            "else",
            "when",
            "case",
            "end",
            "begin",
            "commit",
            "rollback",
            "transaction",
            "grant",
            "revoke",
            "with",
            "recursive",
            "explain",
            "show",
            "describe",
            "use",
            "truncate",
        ]
        .into_iter()
        .collect()
    })
}

fn functions() -> &'static HashSet<&'static str> {
    SQL_FUNCTIONS.get_or_init(|| {
        [
            "COUNT",
            "SUM",
            "AVG",
            "MIN",
            "MAX",
            "ROUND",
            "COALESCE",
            "IFNULL",
            "CONCAT",
            "SUBSTRING",
            "REPLACE",
            "TRIM",
            "UPPER",
            "LOWER",
            "LENGTH",
            "NOW",
            "DATE",
            "YEAR",
            "MONTH",
            "DAY",
            "HOUR",
            "MINUTE",
            "SECOND",
            "CAST",
            "CONVERT",
            "FORMAT",
            "count",
            "sum",
            "avg",
            "min",
            "max",
            "round",
            "coalesce",
            "ifnull",
            "concat",
            "substring",
            "replace",
            "trim",
            "upper",
            "lower",
            "length",
            "now",
            "date",
            "year",
            "month",
            "day",
            "hour",
            "minute",
            "second",
            "cast",
            "convert",
            "format",
            // PostgreSQL specifics
            "pg_size_pretty",
            "pg_database_size",
            "pg_total_relation_size",
            "pg_stat_activity",
        ]
        .into_iter()
        .collect()
    })
}

/// Single-pass SQL tokenizer.
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

        // Line comment: -- ...
        if b == b'-' && i + 1 < len && bytes[i + 1] == b'-' {
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
            if i >= len {
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
            while i < len {
                if bytes[i] == b'\'' {
                    if i + 1 < len && bytes[i + 1] == b'\'' {
                        i += 2; // escaped quote
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            tokens.push(Token {
                range: start..i,
                kind: TokenKind::String,
            });
            continue;
        }

        // Double-quoted identifier
        if b == b'"' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'"' {
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

        // Backtick-quoted identifier (MySQL)
        if b == b'`' {
            let start = i;
            i += 1;
            while i < len && bytes[i] != b'`' {
                i += 1;
            }
            if i < len {
                i += 1;
            }
            tokens.push(Token {
                range: start..i,
                kind: TokenKind::Variable,
            });
            continue;
        }

        // Numbers
        if b.is_ascii_digit() {
            let start = i;
            while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
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
            b'=' | b'<' | b'>' | b'!' | b'+' | b'-' | b'*' | b'/' | b'%'
        ) {
            let start = i;
            i += 1;
            if i < len && (bytes[i] == b'=' || bytes[i] == b'>') {
                i += 1;
            }
            tokens.push(Token {
                range: start..i,
                kind: TokenKind::Operator,
            });
            continue;
        }

        // Semicolons and commas as operators
        if b == b';' || b == b',' {
            tokens.push(Token {
                range: i..i + 1,
                kind: TokenKind::Operator,
            });
            i += 1;
            continue;
        }

        // Words (keywords, functions, identifiers)
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            while i < len
                && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'.')
            {
                i += 1;
            }
            let word = &source[start..i];

            let kind = if keywords().contains(word) {
                TokenKind::Keyword
            } else if functions().contains(word) {
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

        // Variable: @var or @@var (MySQL)
        if b == b'@' {
            let start = i;
            i += 1;
            if i < len && bytes[i] == b'@' {
                i += 1;
            }
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

        i += 1;
    }

    tokens
}
