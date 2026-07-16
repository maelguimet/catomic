//! Purpose: classify basic code tokens within one visible logical line.
//! Owns: Rust/Python/JSON keywords, strings, numbers, and line comments.
//! Must not: parse syntax trees, retain multiline state, emit ANSI, or inspect other lines.
//! Invariants: returned spans are ordered, non-overlapping scalar ranges.
//! Phase: 4-a viewport-only code styling.

use super::{SpanStyle, StyledSpan, SyntaxKind};

pub(super) fn spans(syntax: SyntaxKind, line: &str) -> Vec<StyledSpan> {
    let chars: Vec<char> = line.chars().collect();
    let mut spans = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if is_comment_start(syntax, &chars, index) {
            spans.push(StyledSpan {
                start: index,
                end: chars.len(),
                style: SpanStyle::Comment,
            });
            break;
        }
        if is_quote(syntax, chars[index]) {
            let end = quoted_end(&chars, index);
            spans.push(StyledSpan {
                start: index,
                end,
                style: SpanStyle::String,
            });
            index = end;
            continue;
        }
        if chars[index].is_ascii_digit() {
            let end = token_end(&chars, index, |ch| {
                ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.')
            });
            spans.push(StyledSpan {
                start: index,
                end,
                style: SpanStyle::Number,
            });
            index = end;
            continue;
        }
        if chars[index].is_alphabetic() || chars[index] == '_' {
            let end = token_end(&chars, index, |ch| ch.is_alphanumeric() || ch == '_');
            let token: String = chars[index..end].iter().collect();
            if is_keyword(syntax, &token) {
                spans.push(StyledSpan {
                    start: index,
                    end,
                    style: SpanStyle::Keyword,
                });
            }
            index = end;
            continue;
        }
        index += 1;
    }
    spans
}

fn is_comment_start(syntax: SyntaxKind, chars: &[char], index: usize) -> bool {
    match syntax {
        SyntaxKind::Rust => chars[index..].starts_with(&['/', '/']),
        SyntaxKind::Python => chars[index] == '#',
        _ => false,
    }
}

fn is_quote(syntax: SyntaxKind, ch: char) -> bool {
    ch == '"' || (syntax == SyntaxKind::Python && ch == '\'')
}

fn quoted_end(chars: &[char], start: usize) -> usize {
    let quote = chars[start];
    let mut index = start + 1;
    let mut escaped = false;
    while index < chars.len() {
        let ch = chars[index];
        index += 1;
        if escaped {
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == quote {
            break;
        }
    }
    index
}

fn token_end(chars: &[char], start: usize, accepts: impl Fn(char) -> bool) -> usize {
    start + chars[start..].iter().take_while(|ch| accepts(**ch)).count()
}

fn is_keyword(syntax: SyntaxKind, token: &str) -> bool {
    match syntax {
        SyntaxKind::Rust => RUST_KEYWORDS.contains(&token),
        SyntaxKind::Python => PYTHON_KEYWORDS.contains(&token),
        SyntaxKind::Json => matches!(token, "true" | "false" | "null"),
        SyntaxKind::Plain | SyntaxKind::Markdown | SyntaxKind::MarkdownPreview => false,
    }
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];

const PYTHON_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif",
    "else", "except", "False", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "None", "nonlocal", "not", "or", "pass", "raise", "return", "True", "try", "while",
    "with", "yield",
];
