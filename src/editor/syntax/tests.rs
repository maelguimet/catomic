//! Purpose: specify extension detection and scalar-indexed visible-line syntax spans.
//! Owns: exact Markdown, Rust, Python, JSON, and Unicode fixtures.
//! Must not: render ANSI, touch disk, construct App, or measure performance.
//! Invariants: expected spans are ordered half-open scalar ranges.
//! Phase: 4-a viewport-only syntax foundation.

use super::*;

fn span(start: usize, end: usize, style: SpanStyle) -> StyledSpan {
    StyledSpan { start, end, style }
}

#[test]
fn extensions_select_the_small_builtin_set() {
    assert_eq!(
        syntax_for_path(Some(Path::new("README.md"))),
        SyntaxKind::Markdown
    );
    assert_eq!(
        syntax_for_path(Some(Path::new("main.rs"))),
        SyntaxKind::Rust
    );
    assert_eq!(
        syntax_for_path(Some(Path::new("tool.py"))),
        SyntaxKind::Python
    );
    assert_eq!(
        syntax_for_path(Some(Path::new("data.json"))),
        SyntaxKind::Json
    );
    assert_eq!(
        syntax_for_path(Some(Path::new("notes.txt"))),
        SyntaxKind::Plain
    );
    assert_eq!(syntax_for_path(None), SyntaxKind::Plain);
}

#[test]
fn markdown_styles_headings_markers_fences_and_inline_code() {
    assert_eq!(
        spans_for_line(SyntaxKind::Markdown, "## Heading"),
        vec![span(0, 10, SpanStyle::Heading)]
    );
    assert_eq!(
        spans_for_line(SyntaxKind::Markdown, "- use `cat` now"),
        vec![span(0, 2, SpanStyle::Marker), span(6, 11, SpanStyle::Code)]
    );
    assert_eq!(
        spans_for_line(SyntaxKind::Markdown, "> quote"),
        vec![span(0, 2, SpanStyle::Marker)]
    );
    assert_eq!(
        spans_for_line(SyntaxKind::Markdown, "```rust"),
        vec![span(0, 7, SpanStyle::Code)]
    );
}

#[test]
fn code_lexers_style_keywords_strings_numbers_and_comments() {
    assert_eq!(
        spans_for_line(SyntaxKind::Rust, "let cat = \"猫\"; // note"),
        vec![
            span(0, 3, SpanStyle::Keyword),
            span(10, 13, SpanStyle::String),
            span(15, 22, SpanStyle::Comment)
        ]
    );
    assert_eq!(
        spans_for_line(SyntaxKind::Python, "def cat(x=42): # note"),
        vec![
            span(0, 3, SpanStyle::Keyword),
            span(10, 12, SpanStyle::Number),
            span(15, 21, SpanStyle::Comment)
        ]
    );
    assert_eq!(
        spans_for_line(SyntaxKind::Json, "{\"ok\": true, \"n\": 12}"),
        vec![
            span(1, 5, SpanStyle::String),
            span(7, 11, SpanStyle::Keyword),
            span(13, 16, SpanStyle::String),
            span(18, 20, SpanStyle::Number)
        ]
    );
}

#[test]
fn unicode_before_a_token_keeps_scalar_coordinates() {
    assert_eq!(
        spans_for_line(SyntaxKind::Python, "猫 = 7 # ok"),
        vec![
            span(4, 5, SpanStyle::Number),
            span(6, 10, SpanStyle::Comment)
        ]
    );
}
