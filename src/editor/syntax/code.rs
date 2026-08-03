//! Purpose: classify basic code tokens within one visible logical line.
//! Owns: Rust/Python/JSON keywords, strings, numbers, and line comments.
//! Must not: parse syntax trees, retain multiline state, emit ANSI, or inspect other lines.
//! Invariants: returned spans are ordered, non-overlapping scalar ranges.

use super::{SpanStyle, StyledSpan, SyntaxKind};

pub(super) fn spans(syntax: SyntaxKind, line: &str) -> Vec<StyledSpan> {
    let mut spans = Vec::new();
    let mut skip_until_byte = 0usize;
    if syntax == SyntaxKind::Toml {
        if let Some((start, end)) = toml_table_header(line) {
            spans.push(StyledSpan {
                start,
                end,
                style: SpanStyle::Keyword,
            });
            skip_until_byte = line
                .char_indices()
                .nth(end)
                .map_or(line.len(), |(byte, _)| byte);
        }
    }
    let mut chars = line.char_indices().peekable();
    let mut scalar = 0usize;
    while let Some((byte, ch)) = chars.next() {
        if byte < skip_until_byte {
            scalar = scalar.saturating_add(1);
            continue;
        }
        if is_comment_start(syntax, line, byte, ch) {
            spans.push(StyledSpan {
                start: scalar,
                end: scalar.saturating_add(line[byte..].chars().count()),
                style: SpanStyle::Comment,
            });
            break;
        }
        if is_quote(syntax, ch) {
            let start = scalar;
            scalar = scalar.saturating_add(1);
            let mut escaped = false;
            for (_, quoted) in chars.by_ref() {
                scalar = scalar.saturating_add(1);
                if escaped {
                    escaped = false;
                } else if quoted == '\\' {
                    escaped = true;
                } else if quoted == ch {
                    break;
                }
            }
            spans.push(StyledSpan {
                start,
                end: scalar,
                style: SpanStyle::String,
            });
            continue;
        }
        if ch.is_ascii_digit() {
            let start = scalar;
            scalar = scalar.saturating_add(1);
            take_while(&mut chars, &mut scalar, |next| {
                next.is_ascii_alphanumeric() || matches!(next, '_' | '.')
            });
            spans.push(StyledSpan {
                start,
                end: scalar,
                style: SpanStyle::Number,
            });
            continue;
        }
        if syntax == SyntaxKind::Shell && ch == '$' {
            let start = scalar;
            scalar = scalar.saturating_add(1);
            if chars.peek().is_some_and(|(_, next)| *next == '{') {
                chars.next();
                scalar = scalar.saturating_add(1);
                take_while(&mut chars, &mut scalar, |next| {
                    next.is_alphanumeric() || next == '_'
                });
                if chars.peek().is_some_and(|(_, next)| *next == '}') {
                    chars.next();
                    scalar = scalar.saturating_add(1);
                }
            } else {
                take_while(&mut chars, &mut scalar, |next| {
                    next.is_alphanumeric() || next == '_'
                });
            }
            if scalar > start.saturating_add(1) {
                spans.push(StyledSpan {
                    start,
                    end: scalar,
                    style: SpanStyle::Keyword,
                });
            }
            continue;
        }
        if ch.is_alphabetic() || ch == '_' {
            let start = scalar;
            scalar = scalar.saturating_add(1);
            take_while(&mut chars, &mut scalar, |next| {
                next.is_alphanumeric() || next == '_'
            });
            let byte_end = chars.peek().map_or(line.len(), |(offset, _)| *offset);
            if is_keyword(syntax, &line[byte..byte_end])
                || (syntax == SyntaxKind::Toml && toml_key_ends_at(line, byte_end))
            {
                spans.push(StyledSpan {
                    start,
                    end: scalar,
                    style: SpanStyle::Keyword,
                });
            }
            continue;
        }
        scalar = scalar.saturating_add(1);
    }
    spans
}

fn take_while(
    chars: &mut std::iter::Peekable<std::str::CharIndices<'_>>,
    scalar: &mut usize,
    accepts: impl Fn(char) -> bool,
) {
    while chars.peek().is_some_and(|(_, ch)| accepts(*ch)) {
        chars.next();
        *scalar = (*scalar).saturating_add(1);
    }
}

fn is_comment_start(syntax: SyntaxKind, line: &str, byte: usize, ch: char) -> bool {
    match syntax {
        SyntaxKind::Rust => line[byte..].starts_with("//"),
        SyntaxKind::Python | SyntaxKind::Toml | SyntaxKind::Shell => ch == '#',
        _ => false,
    }
}

fn is_quote(syntax: SyntaxKind, ch: char) -> bool {
    ch == '"'
        || (matches!(
            syntax,
            SyntaxKind::Python | SyntaxKind::Toml | SyntaxKind::Shell
        ) && ch == '\'')
}

fn is_keyword(syntax: SyntaxKind, token: &str) -> bool {
    match syntax {
        SyntaxKind::Rust => RUST_KEYWORDS.contains(&token),
        SyntaxKind::Python => PYTHON_KEYWORDS.contains(&token),
        SyntaxKind::Json => matches!(token, "true" | "false" | "null"),
        SyntaxKind::Toml => matches!(token, "true" | "false"),
        SyntaxKind::Shell => SHELL_KEYWORDS.contains(&token),
        SyntaxKind::Plain
        | SyntaxKind::Unsupported
        | SyntaxKind::Markdown
        | SyntaxKind::MarkdownPreview
        | SyntaxKind::Diff => false,
    }
}

fn toml_key_ends_at(line: &str, byte_end: usize) -> bool {
    line[byte_end..].trim_start().starts_with('=')
}

fn toml_table_header(line: &str) -> Option<(usize, usize)> {
    let leading = line.chars().take_while(|ch| ch.is_whitespace()).count();
    let trimmed = line.trim_start();
    if !trimmed.starts_with('[') {
        return None;
    }
    let closing = trimmed.find(']')?;
    Some((
        leading,
        leading.saturating_add(trimmed[..=closing].chars().count()),
    ))
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

const SHELL_KEYWORDS: &[&str] = &[
    "case", "do", "done", "elif", "else", "esac", "fi", "for", "function", "if", "in", "select",
    "then", "time", "until", "while",
];
