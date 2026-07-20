//! Purpose: compose scalar-indexed syntax spans and semantic active-range styling.
//! Owns: visible-line ANSI color selection, boundary splitting, and reset emission.
//! Must not: query buffers, infer file types, mutate state, or inspect non-visible lines.
//! Invariants: only the supplied visible text is allocated; every styled segment resets ANSI.
//! Phase: 4-a viewport-only syntax styling.

use std::io::{self, Write};

use crate::config::theme::{Color, Style, Theme};
use crate::editor::syntax::{self, HyperlinkSpan, SpanStyle, StyledSpan};
use crate::editor::text_layout;

use super::{ContentSurface, HighlightKind, RenderOptions, TextHighlight};

pub(super) fn write_content_line<W: Write + ?Sized>(
    out: &mut W,
    content: &str,
    row: usize,
    start_col: usize,
    max_cells: usize,
    options: RenderOptions<'_>,
) -> io::Result<()> {
    write_content_line_with_ghost(out, content, row, start_col, max_cells, options, None)
}

pub(super) fn write_content_line_with_ghost<W: Write + ?Sized>(
    out: &mut W,
    content: &str,
    row: usize,
    start_col: usize,
    max_cells: usize,
    options: RenderOptions<'_>,
    ghost: Option<(usize, usize)>,
) -> io::Result<()> {
    let visible_len = text_layout::clipped_scalar_len(content, max_cells);
    let content: String = content.chars().take(visible_len).collect();
    let chars: Vec<char> = content.chars().collect();
    let spans = options.presentation.map_or_else(
        || syntax::spans_for_line(options.syntax, &content),
        |presentation| {
            visible_spans(
                presentation
                    .spans
                    .get(row)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                start_col,
                chars.len(),
            )
        },
    );
    let links = options.presentation.map_or_else(Vec::new, |presentation| {
        visible_links(
            presentation
                .links
                .get(row)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            start_col,
            chars.len(),
        )
    });
    let selected = visible_highlight(options.highlight, row, start_col, chars.len());
    let llm_changed = visible_ranges(
        options.llm_changes.map(|changes| changes.ranges),
        row,
        start_col,
        chars.len(),
    );
    let external_added = visible_ranges(
        options.external_changes.map(|changes| changes.added_ranges),
        row,
        start_col,
        chars.len(),
    );
    let external_changed = visible_ranges(
        options
            .external_changes
            .map(|changes| changes.changed_ranges),
        row,
        start_col,
        chars.len(),
    );
    let boundaries = segment_boundaries(
        &content,
        &spans,
        selected,
        &[&llm_changed, &external_added, &external_changed],
        ghost,
        &links,
    );
    let mut cell = 0;
    for range in boundaries.windows(2) {
        let start = range[0];
        let end = range[1];
        if start == end {
            continue;
        }
        let syntax_styles = spans
            .iter()
            .filter(|span| start >= span.start && start < span.end)
            .map(|span| span.style);
        let hyperlink = links
            .iter()
            .find(|link| start >= link.start && start < link.end)
            .map(|link| link.destination.as_ref());
        let highlighted = selected.is_some_and(|(from, to)| start >= from && start < to);
        let llm_changed = llm_changed
            .iter()
            .any(|(from, to)| start >= *from && start < *to);
        let external_added = external_added
            .iter()
            .any(|(from, to)| start >= *from && start < *to);
        let external_changed = external_changed
            .iter()
            .any(|(from, to)| start >= *from && start < *to);
        let ghost_text = ghost.is_some_and(|(from, to)| start >= from && start < to);
        let segment: String = chars[start..end].iter().collect();
        let style = segment_style(
            options,
            syntax_styles,
            highlighted,
            llm_changed,
            external_added,
            external_changed,
            ghost_text,
        );
        write_segment(
            out,
            &segment,
            style,
            options.whitespace,
            cell,
            options.theme.truecolor,
            hyperlink,
        )?;
        cell = cell.saturating_add(text_layout::cell_width_from(&segment, cell));
    }
    Ok(())
}

fn visible_spans(spans: &[StyledSpan], start_col: usize, content_len: usize) -> Vec<StyledSpan> {
    let visible_end = start_col.saturating_add(content_len);
    spans
        .iter()
        .filter_map(|span| {
            let start = span.start.max(start_col);
            let end = span.end.min(visible_end);
            (start < end).then_some(StyledSpan {
                start: start - start_col,
                end: end - start_col,
                style: span.style,
            })
        })
        .collect()
}

fn visible_links(
    links: &[HyperlinkSpan],
    start_col: usize,
    content_len: usize,
) -> Vec<HyperlinkSpan> {
    let visible_end = start_col.saturating_add(content_len);
    links
        .iter()
        .filter_map(|link| {
            let start = link.start.max(start_col);
            let end = link.end.min(visible_end);
            (start < end).then(|| HyperlinkSpan {
                start: start - start_col,
                end: end - start_col,
                destination: link.destination.clone(),
            })
        })
        .collect()
}

fn visible_ranges(
    ranges: Option<&[TextHighlight]>,
    row: usize,
    start_col: usize,
    content_len: usize,
) -> Vec<(usize, usize)> {
    ranges
        .into_iter()
        .flat_map(|ranges| ranges.iter().copied())
        .filter_map(|range| visible_highlight(Some(range), row, start_col, content_len))
        .collect()
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
    change_sets: &[&[(usize, usize)]],
    ghost: Option<(usize, usize)>,
    links: &[HyperlinkSpan],
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
    for changed in change_sets {
        for &(start, end) in *changed {
            boundaries.push(start);
            boundaries.push(end);
        }
    }
    if let Some((start, end)) = ghost {
        boundaries.push(start.min(content_len));
        boundaries.push(end.min(content_len));
    }
    for link in links {
        boundaries.push(link.start.min(content_len));
        boundaries.push(link.end.min(content_len));
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
    style: Style,
    whitespace: bool,
    initial_cell: usize,
    truecolor: bool,
    hyperlink: Option<&str>,
) -> io::Result<()> {
    let text = text_layout::expand_tabs(text, whitespace, initial_cell);
    if let Some(destination) = hyperlink {
        write!(out, "\x1b]8;;{destination}\x1b\\")?;
    }
    write_styled_text(out, &text, style, truecolor)?;
    if hyperlink.is_some() {
        write!(out, "\x1b]8;;\x1b\\")?;
    }
    Ok(())
}

fn segment_style(
    options: RenderOptions<'_>,
    spans: impl Iterator<Item = SpanStyle>,
    highlighted: bool,
    llm_changed: bool,
    external_added: bool,
    external_changed: bool,
    ghost: bool,
) -> Style {
    let theme = options.theme;
    let mut style = match options.surface {
        ContentSurface::Normal => theme.text,
        ContentSurface::Preview | ContentSurface::Diff => theme.text.overlay(theme.preview),
    };
    for span in spans {
        style = style.overlay(span_style(theme, span));
    }
    if external_added {
        style = style.overlay(theme.external_added);
    }
    if external_changed {
        style = style.overlay(theme.external_changed);
    }
    if llm_changed {
        style = style.overlay(theme.llm_changed);
    }
    if ghost {
        style = style.overlay(theme.autocomplete);
    }
    if highlighted {
        style = style.overlay(match options.highlight_kind {
            HighlightKind::Selection => theme.selection,
            HighlightKind::Search => theme.search_match,
        });
    }
    style
}

fn span_style(theme: Theme, style: SpanStyle) -> Style {
    match style {
        SpanStyle::Heading => theme.markdown_heading,
        SpanStyle::Marker => theme.markdown_marker,
        SpanStyle::Link => theme.markdown_link,
        SpanStyle::Keyword => theme.syntax_keyword,
        SpanStyle::String => theme.syntax_string,
        SpanStyle::Comment => theme.syntax_comment,
        SpanStyle::Number => theme.syntax_number,
        SpanStyle::Code => theme.markdown_code,
        SpanStyle::PreviewCode => Style {
            reversed: Some(true),
            ..theme.markdown_code
        },
        SpanStyle::PreviewHeading4 => Style {
            bold: Some(false),
            underlined: Some(true),
            ..theme.markdown_heading
        },
        SpanStyle::PreviewHeading5 => Style {
            bold: Some(false),
            ..theme.markdown_heading
        },
        SpanStyle::PreviewHeading6 => Style {
            bold: Some(false),
            dim: Some(true),
            ..theme.markdown_heading
        },
        SpanStyle::PreviewLink => Style {
            underlined: Some(true),
            ..theme.markdown_link
        },
        SpanStyle::Emphasis => theme.markdown_emphasis,
        SpanStyle::PreviewStrong => Style {
            bold: Some(true),
            ..theme.markdown_emphasis
        },
        SpanStyle::PreviewEmphasis => Style {
            underlined: Some(true),
            ..theme.markdown_emphasis
        },
        SpanStyle::PreviewStrikethrough => Style {
            crossed_out: Some(true),
            ..theme.markdown_emphasis
        },
        SpanStyle::DiffAdded => theme.diff_added,
        SpanStyle::DiffRemoved => theme.diff_removed,
    }
}

pub(super) fn write_row_start<W: Write + ?Sized>(
    out: &mut W,
    row: usize,
    style: Style,
    truecolor: bool,
) -> io::Result<()> {
    write!(out, "\x1b[{row};1H")?;
    if write_style_prefix(out, style, truecolor)? {
        write!(out, "\x1b[K\x1b[0m")
    } else {
        write!(out, "\x1b[K")
    }
}

pub(super) fn write_styled_text<W: Write + ?Sized>(
    out: &mut W,
    text: &str,
    style: Style,
    truecolor: bool,
) -> io::Result<()> {
    if write_style_prefix(out, style, truecolor)? {
        write!(out, "{text}\x1b[0m")
    } else {
        write!(out, "{text}")
    }
}

fn write_style_prefix<W: Write + ?Sized>(
    out: &mut W,
    style: Style,
    truecolor: bool,
) -> io::Result<bool> {
    let mut codes = Vec::new();
    if let Some(color) = style.fg {
        codes.push(color_code(color, true, truecolor));
    }
    if let Some(color) = style.bg {
        codes.push(color_code(color, false, truecolor));
    }
    if style.bold == Some(true) {
        codes.push("1".to_string());
    }
    if style.dim == Some(true) {
        codes.push("2".to_string());
    }
    if style.underlined == Some(true) {
        codes.push("4".to_string());
    }
    if style.reversed == Some(true) {
        codes.push("7".to_string());
    }
    if style.crossed_out == Some(true) {
        codes.push("9".to_string());
    }
    if codes.is_empty() {
        return Ok(false);
    }
    write!(out, "\x1b[{}m", codes.join(";"))?;
    Ok(true)
}

fn color_code(color: Color, foreground: bool, truecolor: bool) -> String {
    let base = if foreground { 30 } else { 40 };
    match color {
        Color::Default => if foreground { "39" } else { "49" }.to_string(),
        Color::Ansi(index) if index < 8 => (base + u16::from(index)).to_string(),
        Color::Ansi(index) => (base + 60 + u16::from(index - 8)).to_string(),
        Color::Indexed(index) => format!("{};5;{index}", if foreground { 38 } else { 48 }),
        Color::Rgb(red, green, blue) if truecolor => {
            format!(
                "{};2;{red};{green};{blue}",
                if foreground { 38 } else { 48 }
            )
        }
        Color::Rgb(red, green, blue) => {
            let index = crate::config::theme::indexed_fallback(red, green, blue);
            format!("{};5;{index}", if foreground { 38 } else { 48 })
        }
    }
}

pub(super) fn write_cursor_color<W: Write + ?Sized>(out: &mut W, theme: Theme) -> io::Result<()> {
    let Some(color) = theme.cursor else {
        return Ok(());
    };
    if color == Color::Default {
        return write!(out, "\x1b]112\x07");
    }
    let (red, green, blue) = color_rgb(color);
    write!(out, "\x1b]12;#{red:02x}{green:02x}{blue:02x}\x07")
}

fn color_rgb(color: Color) -> (u8, u8, u8) {
    const ANSI: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    match color {
        Color::Default => (255, 255, 255),
        Color::Ansi(index) => ANSI[index.min(15) as usize],
        Color::Rgb(red, green, blue) => (red, green, blue),
        Color::Indexed(index) if index < 16 => ANSI[index as usize],
        Color::Indexed(index) if index < 232 => {
            let offset = index - 16;
            let level = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
            (
                level(offset / 36),
                level((offset / 6) % 6),
                level(offset % 6),
            )
        }
        Color::Indexed(index) => {
            let level = 8 + (index - 232) * 10;
            (level, level, level)
        }
    }
}

pub(super) fn write_semantic_gutter<W: Write + ?Sized>(
    out: &mut W,
    style: Style,
    truecolor: bool,
) -> io::Result<()> {
    let marker = if style.fg.is_some() || style.bg.is_some() {
        "┃"
    } else {
        "!"
    };
    let style = style.overlay(Style {
        bold: Some(true),
        ..Style::default()
    });
    write_styled_text(out, marker, style, truecolor)?;
    write!(out, " ")
}

#[cfg(test)]
mod tests;
