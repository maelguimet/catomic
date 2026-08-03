//! Purpose: classify file syntax and produce scalar-indexed styles for one visible line.
//! Owns: extension detection and pure Markdown/code/config lexical spans.
//! Must not: emit ANSI, read files/buffers, retain caches, mutate state, or scan other lines.
//! Invariants: spans are ordered, non-overlapping, and use half-open Unicode scalar indices.

use std::path::Path;
use std::sync::Arc;

mod code;
mod markdown;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum SyntaxKind {
    #[default]
    Plain,
    Unsupported,
    Markdown,
    MarkdownPreview,
    Rust,
    Python,
    Json,
    Toml,
    Shell,
    Diff,
}

const MAX_LEX_SCALARS: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    PreviewInlineCode,
    PreviewCodeBlock,
    PreviewHeading1,
    PreviewHeading2,
    PreviewHeading3,
    PreviewHeading4,
    PreviewHeading5,
    PreviewHeading6,
    PreviewLink,
    PreviewStrong,
    PreviewEmphasis,
    PreviewStrikethrough,
    DiffAdded,
    DiffRemoved,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct HyperlinkSpan {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) destination: Arc<str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct StyledSpan {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) style: SpanStyle,
}

pub(crate) fn syntax_for_path(path: Option<&Path>) -> SyntaxKind {
    let filename = path
        .and_then(Path::file_name)
        .and_then(|filename| filename.to_str())
        .map(str::to_ascii_lowercase);
    if filename.as_deref().is_some_and(|filename| {
        matches!(
            filename,
            ".bashrc"
                | ".bash_profile"
                | ".bash_login"
                | ".profile"
                | ".zshrc"
                | ".zprofile"
                | ".zlogin"
                | ".kshrc"
        )
    }) {
        return SyntaxKind::Shell;
    }
    match path
        .and_then(Path::extension)
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("txt" | "text" | "log") => SyntaxKind::Plain,
        Some("md" | "markdown" | "mkd") => SyntaxKind::Markdown,
        Some("rs") => SyntaxKind::Rust,
        Some("py" | "pyw") => SyntaxKind::Python,
        Some("json") => SyntaxKind::Json,
        Some("toml") => SyntaxKind::Toml,
        Some("sh" | "bash" | "zsh" | "ksh" | "fish") => SyntaxKind::Shell,
        Some("diff" | "patch") => SyntaxKind::Diff,
        Some(_) => SyntaxKind::Unsupported,
        None => SyntaxKind::Plain,
    }
}

pub(crate) fn spans_for_line(syntax: SyntaxKind, line: &str) -> Vec<StyledSpan> {
    match syntax {
        SyntaxKind::Plain | SyntaxKind::Unsupported => Vec::new(),
        SyntaxKind::Markdown => markdown::spans(bounded_prefix(line)),
        SyntaxKind::MarkdownPreview => Vec::new(),
        SyntaxKind::Rust
        | SyntaxKind::Python
        | SyntaxKind::Json
        | SyntaxKind::Toml
        | SyntaxKind::Shell => code::spans(syntax, bounded_prefix(line)),
        SyntaxKind::Diff => diff_spans(bounded_prefix(line)),
    }
}

pub(crate) const fn syntax_name(syntax: SyntaxKind) -> &'static str {
    match syntax {
        SyntaxKind::Plain => "plain text",
        SyntaxKind::Unsupported => "unsupported",
        SyntaxKind::Markdown | SyntaxKind::MarkdownPreview => "Markdown",
        SyntaxKind::Rust => "Rust",
        SyntaxKind::Python => "Python",
        SyntaxKind::Json => "JSON",
        SyntaxKind::Toml => "TOML",
        SyntaxKind::Shell => "shell",
        SyntaxKind::Diff => "diff",
    }
}

fn bounded_prefix(line: &str) -> &str {
    if line.len() <= MAX_LEX_SCALARS {
        return line;
    }
    line.char_indices()
        .nth(MAX_LEX_SCALARS)
        .map_or(line, |(byte, _)| &line[..byte])
}

fn diff_spans(line: &str) -> Vec<StyledSpan> {
    let style = if line.starts_with('+') && !line.starts_with("+++") {
        Some(SpanStyle::DiffAdded)
    } else if line.starts_with('-') && !line.starts_with("---") {
        Some(SpanStyle::DiffRemoved)
    } else {
        None
    };
    style
        .map(|style| {
            vec![StyledSpan {
                start: 0,
                end: line.chars().count(),
                style,
            }]
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
