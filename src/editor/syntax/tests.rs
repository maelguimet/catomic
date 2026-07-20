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
    assert_eq!(
        spans_for_line(SyntaxKind::Markdown, "read *this* now"),
        vec![span(5, 11, SpanStyle::Emphasis)]
    );
    assert_eq!(
        spans_for_line(SyntaxKind::Markdown, "read **bold** now"),
        vec![span(5, 13, SpanStyle::Emphasis)]
    );
}

#[test]
fn diff_styles_only_content_additions_and_removals() {
    assert_eq!(
        spans_for_line(SyntaxKind::Diff, "+added 猫"),
        vec![span(0, 8, SpanStyle::DiffAdded)]
    );
    assert_eq!(
        spans_for_line(SyntaxKind::Diff, "-removed"),
        vec![span(0, 8, SpanStyle::DiffRemoved)]
    );
    assert!(spans_for_line(SyntaxKind::Diff, "+++ b/file").is_empty());
    assert!(spans_for_line(SyntaxKind::Diff, "--- a/file").is_empty());
}

#[test]
fn markdown_styles_an_empty_atx_heading_without_changing_its_range() {
    assert_eq!(
        spans_for_line(SyntaxKind::Markdown, "###"),
        vec![span(0, 3, SpanStyle::Heading)]
    );
    assert!(spans_for_line(SyntaxKind::Markdown, "#######").is_empty());
}

#[test]
fn markdown_styles_emphasis_links_tasks_and_table_delimiters() {
    let line = "- [x] **猫** and *é* [link](https://example.com) | `code|x` |";
    let spans = spans_for_line(SyntaxKind::Markdown, line);
    let chars: Vec<char> = line.chars().collect();
    let styled: Vec<(String, SpanStyle)> = spans
        .iter()
        .map(|span| (chars[span.start..span.end].iter().collect(), span.style))
        .collect();

    assert_eq!(
        styled,
        vec![
            ("- ".to_string(), SpanStyle::Marker),
            ("[x] ".to_string(), SpanStyle::Marker),
            ("**猫**".to_string(), SpanStyle::Emphasis),
            ("*é*".to_string(), SpanStyle::Emphasis),
            ("[link](https://example.com)".to_string(), SpanStyle::Link),
            ("|".to_string(), SpanStyle::Marker),
            ("`code|x`".to_string(), SpanStyle::Code),
            ("|".to_string(), SpanStyle::Marker),
        ]
    );
}

#[test]
fn markdown_table_alignment_row_and_escaped_pipe_keep_scalar_ranges() {
    assert_eq!(
        spans_for_line(SyntaxKind::Markdown, "| :--- | :----: | ----: |"),
        vec![
            span(0, 1, SpanStyle::Marker),
            span(2, 6, SpanStyle::Marker),
            span(7, 8, SpanStyle::Marker),
            span(9, 15, SpanStyle::Marker),
            span(16, 17, SpanStyle::Marker),
            span(18, 23, SpanStyle::Marker),
            span(24, 25, SpanStyle::Marker),
        ]
    );
    assert_eq!(
        spans_for_line(SyntaxKind::Markdown, r"| a\|b |"),
        vec![span(0, 1, SpanStyle::Marker), span(7, 8, SpanStyle::Marker)]
    );
}

#[test]
fn markdown_code_runs_match_only_equal_complete_delimiters() {
    assert_eq!(
        spans_for_line(SyntaxKind::Markdown, "``a`b``"),
        vec![span(0, 7, SpanStyle::Code)]
    );
    assert!(spans_for_line(SyntaxKind::Markdown, "`open `` still").is_empty());
}

#[test]
fn markdown_preview_styles_rendered_headings_markers_and_code() {
    assert_eq!(
        spans_for_line(SyntaxKind::MarkdownPreview, "# Heading"),
        vec![span(0, 9, SpanStyle::Heading)]
    );
    assert_eq!(
        spans_for_line(SyntaxKind::MarkdownPreview, "- use `cat`"),
        vec![span(0, 2, SpanStyle::Marker), span(6, 11, SpanStyle::Code)]
    );
    assert_eq!(
        spans_for_line(SyntaxKind::MarkdownPreview, "12. item"),
        vec![span(0, 4, SpanStyle::Marker)]
    );
    assert_eq!(
        spans_for_line(SyntaxKind::MarkdownPreview, "    let cat = 1;"),
        vec![span(0, 16, SpanStyle::Code)]
    );
    assert_eq!(
        spans_for_line(SyntaxKind::MarkdownPreview, "link <https://example.com>"),
        vec![span(5, 26, SpanStyle::Link)]
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
