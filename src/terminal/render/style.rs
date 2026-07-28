//! Purpose: compose scalar-indexed syntax spans and semantic active-range styling.
//! Owns: visible-line ANSI color selection, boundary splitting, and reset emission.
//! Must not: query buffers, infer file types, mutate state, or inspect non-visible lines.
//! Invariants: only the supplied visible text is allocated; every styled segment resets ANSI.

use std::io::{self, Write};

use crate::config::theme::{Color, Style, Theme};
use crate::editor::syntax::{self, HyperlinkSpan, SpanStyle, StyledSpan};
use crate::editor::text_layout;

use super::{ContentSurface, HighlightKind, RenderOptions, TextHighlight};

#[cfg(test)]
pub(super) fn write_content_line<W: Write + ?Sized>(
    out: &mut W,
    content: &str,
    row: usize,
    start_col: usize,
    max_cells: usize,
    options: RenderOptions<'_>,
) -> io::Result<()> {
    let mut layout = text_layout::VisibleLineLayout::default();
    layout.build(content, max_cells);
    let mut boundaries = Vec::new();
    write_content_line_from_layout(
        out,
        content,
        row,
        start_col,
        options,
        &layout,
        &mut boundaries,
    )
}

pub(super) fn write_content_line_from_layout<W: Write + ?Sized>(
    out: &mut W,
    content: &str,
    row: usize,
    start_col: usize,
    options: RenderOptions<'_>,
    layout: &text_layout::VisibleLineLayout,
    boundaries: &mut Vec<usize>,
) -> io::Result<()> {
    let content = &content[..layout.byte_len()];
    let content_len = layout.scalar_len();
    let spans = options.presentation.map_or_else(
        || syntax::spans_for_line(options.syntax, content),
        |presentation| {
            visible_spans(
                presentation
                    .spans
                    .get(row)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                start_col,
                content_len,
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
            content_len,
        )
    });
    let selected = visible_highlight(options.highlight, row, start_col, content_len);
    let lint = visible_ranges(options.lint_ranges, row, start_col, content_len);
    let external_added = visible_ranges(
        options.external_changes.map(|changes| changes.added_ranges),
        row,
        start_col,
        content_len,
    );
    let external_changed = visible_ranges(
        options
            .external_changes
            .map(|changes| changes.changed_ranges),
        row,
        start_col,
        content_len,
    );
    segment_boundaries(
        layout,
        &spans,
        selected,
        &[&lint, &external_added, &external_changed],
        &links,
        boundaries,
    );
    for range in boundaries.windows(2) {
        let start = range[0];
        let end = range[1];
        if start == end {
            continue;
        }
        let syntax_styles = spans
            .iter()
            .filter(|span| ranges_overlap(start, end, span.start, span.end))
            .map(|span| span.style);
        let hyperlink = links
            .iter()
            .find(|link| ranges_overlap(start, end, link.start, link.end))
            .map(|link| link.destination);
        let highlighted = selected.is_some_and(|(from, to)| ranges_overlap(start, end, from, to));
        let lint = lint
            .iter()
            .any(|(from, to)| ranges_overlap(start, end, *from, *to));
        let external_added = external_added
            .iter()
            .any(|(from, to)| ranges_overlap(start, end, *from, *to));
        let external_changed = external_changed
            .iter()
            .any(|(from, to)| ranges_overlap(start, end, *from, *to));
        let style = segment_style(
            options,
            syntax_styles,
            SegmentRoles {
                highlighted,
                lint,
                external_added,
                external_changed,
            },
        );
        write_segment(
            out,
            LayoutRange {
                text: content,
                layout,
                scalar_start: start,
                scalar_end: end,
            },
            style,
            options.whitespace,
            options.theme.truecolor,
            hyperlink,
        )?;
    }
    Ok(())
}

fn ranges_overlap(
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
) -> bool {
    left_start < right_end && right_start < left_end
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

#[derive(Clone, Copy)]
struct VisibleLink<'a> {
    start: usize,
    end: usize,
    destination: &'a str,
}

fn visible_links<'a>(
    links: &'a [HyperlinkSpan],
    start_col: usize,
    content_len: usize,
) -> Vec<VisibleLink<'a>> {
    let visible_end = start_col.saturating_add(content_len);
    links
        .iter()
        .filter_map(|link| {
            let start = link.start.max(start_col);
            let end = link.end.min(visible_end);
            (start < end).then(|| VisibleLink {
                start: start - start_col,
                end: end - start_col,
                destination: link.destination.as_ref(),
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
        .flat_map(|ranges| ranges_for_row(ranges, row).iter().copied())
        .filter_map(|range| visible_highlight(Some(range), row, start_col, content_len))
        .collect()
}

/// Stored render annotations are sorted by their single-line coordinates at
/// installation. This returns only ranges that can overlap `row`; end-column
/// zero remains half-open and therefore does not paint its ending row.
fn ranges_for_row(ranges: &[TextHighlight], row: usize) -> &[TextHighlight] {
    let first = ranges.partition_point(|range| {
        range.end.row < row || (range.end.row == row && range.end.col == 0)
    });
    let end = first + ranges[first..].partition_point(|range| range.start.row <= row);
    &ranges[first..end]
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
    layout: &text_layout::VisibleLineLayout,
    spans: &[StyledSpan],
    selected: Option<(usize, usize)>,
    change_sets: &[&[(usize, usize)]],
    links: &[VisibleLink<'_>],
    boundaries: &mut Vec<usize>,
) {
    let content_len = layout.scalar_len();
    boundaries.clear();
    boundaries.extend([0, content_len]);
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
    for link in links {
        boundaries.push(link.start.min(content_len));
        boundaries.push(link.end.min(content_len));
    }
    boundaries.sort_unstable();
    for boundary in boundaries.iter_mut() {
        *boundary = layout.snap_scalar(*boundary);
    }
    boundaries.push(content_len);
    boundaries.sort_unstable();
    boundaries.dedup();
}

#[derive(Clone, Copy)]
struct LayoutRange<'a> {
    text: &'a str,
    layout: &'a text_layout::VisibleLineLayout,
    scalar_start: usize,
    scalar_end: usize,
}

fn write_segment<W: Write + ?Sized>(
    out: &mut W,
    range: LayoutRange<'_>,
    style: Style,
    whitespace: bool,
    truecolor: bool,
    hyperlink: Option<&str>,
) -> io::Result<()> {
    if let Some(destination) = hyperlink {
        write!(out, "\x1b]8;;{destination}\x1b\\")?;
    }
    let styled = write_style_prefix(out, style, truecolor)?;
    write_layout_range(out, range, whitespace)?;
    if styled {
        write!(out, "\x1b[0m")?;
    }
    if hyperlink.is_some() {
        write!(out, "\x1b]8;;\x1b\\")?;
    }
    Ok(())
}

fn write_layout_range<W: Write + ?Sized>(
    out: &mut W,
    range: LayoutRange<'_>,
    whitespace: bool,
) -> io::Result<()> {
    let LayoutRange {
        text,
        layout,
        scalar_start,
        scalar_end,
    } = range;
    let graphemes = layout.grapheme_range(scalar_start, scalar_end);
    let transformed = graphemes.iter().any(|grapheme| {
        grapheme.is_tab || grapheme.has_control || (whitespace && grapheme.is_space)
    });
    if !transformed {
        let byte_start = layout.boundary_byte(scalar_start);
        let byte_end = layout.boundary_byte(scalar_end);
        return out.write_all(&text.as_bytes()[byte_start..byte_end]);
    }

    let mut run_start = None;
    for grapheme in graphemes {
        let needs_transform =
            grapheme.is_tab || grapheme.has_control || (whitespace && grapheme.is_space);
        if !needs_transform {
            run_start.get_or_insert(grapheme.byte_start);
            continue;
        }
        if let Some(byte_start) = run_start.take() {
            out.write_all(&text.as_bytes()[byte_start..grapheme.byte_start])?;
        }
        if grapheme.is_tab {
            if whitespace {
                write!(out, "→")?;
            }
            let marker_width = usize::from(whitespace);
            for _ in marker_width..grapheme.cell_end.saturating_sub(grapheme.cell_start) {
                out.write_all(b" ")?;
            }
        } else if whitespace && grapheme.is_space {
            write!(out, "·")?;
        } else {
            for ch in text[grapheme.byte_start..grapheme.byte_end].chars() {
                let safe = text_layout::terminal_safe_char(ch);
                let mut encoded = [0; 4];
                out.write_all(safe.encode_utf8(&mut encoded).as_bytes())?;
            }
        }
    }
    if let Some(byte_start) = run_start {
        let byte_end = graphemes
            .last()
            .map_or(byte_start, |grapheme| grapheme.byte_end);
        out.write_all(&text.as_bytes()[byte_start..byte_end])?;
    }
    Ok(())
}

struct SegmentRoles {
    highlighted: bool,
    lint: bool,
    external_added: bool,
    external_changed: bool,
}

fn segment_style(
    options: RenderOptions<'_>,
    spans: impl Iterator<Item = SpanStyle>,
    roles: SegmentRoles,
) -> Style {
    let theme = options.theme;
    let mut style = match options.surface {
        ContentSurface::Normal => theme.text,
        ContentSurface::Preview => theme.text.overlay(theme.preview),
    };
    for span in spans {
        style = style.overlay(span_style(theme, span));
    }
    if roles.external_added {
        style = style.overlay(theme.external_added);
    }
    if roles.external_changed {
        style = style.overlay(theme.external_changed);
    }
    if roles.lint {
        style = style.overlay(theme.lint);
    }
    if roles.highlighted {
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
        SpanStyle::PreviewInlineCode => Style {
            bold: Some(true),
            ..theme.markdown_code
        },
        SpanStyle::PreviewCodeBlock => theme.markdown_code,
        SpanStyle::PreviewHeading1 => Style {
            bold: Some(true),
            reversed: Some(true),
            ..theme.markdown_heading
        },
        SpanStyle::PreviewHeading2 => Style {
            bold: Some(true),
            ..theme.markdown_heading
        },
        SpanStyle::PreviewHeading3 => Style {
            bold: Some(false),
            ..theme.markdown_heading
        },
        SpanStyle::PreviewHeading4 => Style {
            bold: Some(false),
            ..theme.markdown_heading
        },
        SpanStyle::PreviewHeading5 => Style {
            bold: Some(false),
            dim: Some(true),
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

#[cfg(test)]
mod tests;
