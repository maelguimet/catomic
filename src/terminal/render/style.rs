//! Purpose: compose scalar-indexed syntax spans and active-range reverse video.
//! Owns: visible-line ANSI color selection, boundary splitting, and reset emission.
//! Must not: query buffers, infer file types, mutate state, or inspect non-visible lines.
//! Invariants: only the supplied visible text is allocated; every styled segment resets ANSI.
//! Phase: 4-a viewport-only syntax styling.

use std::io::{self, Write};

use crate::editor::syntax::{self, SpanStyle, StyledSpan};
use crate::editor::text_layout;

use super::{RenderOptions, TextHighlight};

pub(super) fn write_content_line<W: Write + ?Sized>(
    out: &mut W,
    content: &str,
    row: usize,
    start_col: usize,
    max_cells: usize,
    options: RenderOptions,
) -> io::Result<()> {
    let visible_len = text_layout::clipped_scalar_len(content, max_cells);
    let content: String = content.chars().take(visible_len).collect();
    let chars: Vec<char> = content.chars().collect();
    let spans = syntax::spans_for_line(options.syntax, &content);
    let selected = visible_highlight(options.highlight, row, start_col, chars.len());
    let boundaries = segment_boundaries(&content, &spans, selected);
    let mut cell = 0;
    for range in boundaries.windows(2) {
        let start = range[0];
        let end = range[1];
        if start == end {
            continue;
        }
        let style = spans
            .iter()
            .find(|span| start >= span.start && start < span.end)
            .map(|span| span.style);
        let reverse = selected.is_some_and(|(from, to)| start >= from && start < to);
        let segment: String = chars[start..end].iter().collect();
        write_segment(out, &segment, style, reverse, options.whitespace, cell)?;
        cell = cell.saturating_add(text_layout::cell_width_from(&segment, cell));
    }
    Ok(())
}

fn visible_highlight(
    highlight: Option<TextHighlight>,
    row: usize,
    start_col: usize,
    content_len: usize,
) -> Option<(usize, usize)> {
    let highlight = highlight.filter(|highlight| {
        row >= highlight.start.row
            && row <= highlight.end.row
            && !(row == highlight.end.row && highlight.end.col == 0)
    })?;
    let visible_end = start_col.saturating_add(content_len);
    let range_start = if row == highlight.start.row {
        highlight.start.col
    } else {
        0
    };
    let range_end = if row == highlight.end.row {
        highlight.end.col
    } else {
        usize::MAX
    };
    let start = range_start.max(start_col);
    let end = range_end.min(visible_end);
    (start < end).then_some((start - start_col, end - start_col))
}

fn segment_boundaries(
    content: &str,
    spans: &[StyledSpan],
    selected: Option<(usize, usize)>,
) -> Vec<usize> {
    let content_len = content.chars().count();
    let mut boundaries = vec![0, content_len];
    for span in spans {
        boundaries.push(span.start.min(content_len));
        boundaries.push(span.end.min(content_len));
    }
    if let Some((start, end)) = selected {
        boundaries.push(start);
        boundaries.push(end);
    }
    boundaries.sort_unstable();
    boundaries = boundaries
        .into_iter()
        .map(|col| text_layout::snap_to_grapheme_col(content, col))
        .collect();
    boundaries.push(content_len);
    boundaries.sort_unstable();
    boundaries.dedup();
    boundaries
}

fn write_segment<W: Write + ?Sized>(
    out: &mut W,
    text: &str,
    style: Option<SpanStyle>,
    reverse: bool,
    whitespace: bool,
    initial_cell: usize,
) -> io::Result<()> {
    let text = text_layout::expand_tabs(text, whitespace, initial_cell);
    let code = style.map(style_code);
    match (code, reverse) {
        (None, false) => write!(out, "{text}"),
        (None, true) => write!(out, "\x1b[7m{text}\x1b[27m"),
        (Some(code), false) => write!(out, "\x1b[{code}m{text}\x1b[0m"),
        (Some(code), true) => write!(out, "\x1b[{code};7m{text}\x1b[0m"),
    }
}

fn style_code(style: SpanStyle) -> &'static str {
    match style {
        SpanStyle::Heading => "1;36",
        SpanStyle::Marker => "36",
        SpanStyle::Keyword => "35",
        SpanStyle::String => "32",
        SpanStyle::Comment => "2;90",
        SpanStyle::Number => "33",
        SpanStyle::Code => "36",
    }
}

#[cfg(test)]
mod tests;
