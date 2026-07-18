//! Purpose: classify file syntax and produce scalar-indexed styles for one visible line.
//! Owns: extension detection and pure Markdown/Rust/Python/JSON lexical spans.
//! Must not: emit ANSI, read files/buffers, retain caches, mutate state, or scan other lines.
//! Invariants: spans are ordered, non-overlapping, and use half-open Unicode scalar indices.
//! Phase: 4-a viewport-only syntax foundation.

use std::path::Path;

mod code;
mod markdown;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SyntaxKind {
    #[default]
    Plain,
    Markdown,
    MarkdownPreview,
    Rust,
    Python,
    Json,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpanStyle {
    Heading,
    Marker,
    Emphasis,
    Link,
    Keyword,
    String,
    Comment,
    Number,
    Code,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StyledSpan {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) style: SpanStyle,
}

pub(crate) fn syntax_for_path(path: Option<&Path>) -> SyntaxKind {
    match path
        .and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("md" | "markdown" | "mkd") => SyntaxKind::Markdown,
        Some("rs") => SyntaxKind::Rust,
        Some("py" | "pyw") => SyntaxKind::Python,
        Some("json") => SyntaxKind::Json,
        _ => SyntaxKind::Plain,
    }
}

pub(crate) fn spans_for_line(syntax: SyntaxKind, line: &str) -> Vec<StyledSpan> {
    match syntax {
        SyntaxKind::Plain => Vec::new(),
        SyntaxKind::Markdown => markdown::spans(line),
        SyntaxKind::MarkdownPreview => markdown::preview_spans(line),
        SyntaxKind::Rust | SyntaxKind::Python | SyntaxKind::Json => code::spans(syntax, line),
    }
}

#[cfg(test)]
mod tests;
