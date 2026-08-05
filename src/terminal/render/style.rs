//! Purpose: compose scalar-indexed syntax spans and semantic active-range styling.
//! Owns: visible-line ANSI color selection, boundary splitting, and reset emission.
//! Must not: query buffers, infer file types, mutate state, or inspect non-visible lines.
//! Invariants: style transitions allocate nothing; rows and hyperlinks reset conservatively.

use std::fmt::Display;
use std::io::{self, Write};
use std::sync::Arc;

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
        |presentation| visible_spans(presentation.annotations.spans(row), start_col, content_len),
    );
    let links = options.presentation.map_or_else(
        || visible_links(syntax::hyperlinks_for_line(content), 0, content_len),
        |presentation| visible_links(presentation.annotations.links(row), start_col, content_len),
    );
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
    let mut style_state = StyleState::default();
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
            .map(|link| link.destination.as_ref());
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
            SegmentOutput {
                style,
                whitespace: options.whitespace,
                truecolor: options.theme.truecolor,
                hyperlink,
            },
            &mut style_state,
        )?;
    }
    style_state.reset(out)
}

fn ranges_overlap(
    left_start: usize,
    left_end: usize,
    right_start: usize,
    right_end: usize,
) -> bool {
    left_start < right_end && right_start < left_end
}

fn visible_spans(
    spans: impl IntoIterator<Item = StyledSpan>,
    start_col: usize,
    content_len: usize,
) -> Vec<StyledSpan> {
    let visible_end = start_col.saturating_add(content_len);
    spans
        .into_iter()
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

#[derive(Clone)]
struct VisibleLink {
    start: usize,
    end: usize,
    destination: Arc<str>,
}

fn visible_links(
    links: impl IntoIterator<Item = HyperlinkSpan>,
    start_col: usize,
    content_len: usize,
) -> Vec<VisibleLink> {
    let visible_end = start_col.saturating_add(content_len);
    links
        .into_iter()
        .filter_map(|link| {
            let start = link.start.max(start_col);
            let end = link.end.min(visible_end);
            (start < end).then(|| VisibleLink {
                start: start - start_col,
                end: end - start_col,
                destination: link.destination,
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
pub(super) fn ranges_for_row(ranges: &[TextHighlight], row: usize) -> &[TextHighlight] {
    let first = ranges.partition_point(|range| {
        range.end.row < row || (range.end.row == row && range.end.col == 0)
    });
    let end = first + ranges[first..].partition_point(|range| range.start.row <= row);
    &ranges[first..end]
}

pub(super) fn visible_highlight(
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
    links: &[VisibleLink],
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
    segment: SegmentOutput<'_>,
    style_state: &mut StyleState,
) -> io::Result<()> {
    if let Some(destination) = segment.hyperlink {
        style_state.force_reset(out)?;
        write!(out, "\x1b]8;;{destination}\x1b\\")?;
        style_state.transition(out, segment.style, segment.truecolor)?;
        write_layout_range(out, range, segment.whitespace)?;
        style_state.force_reset(out)?;
        write!(out, "\x1b]8;;\x1b\\")
    } else {
        style_state.transition(out, segment.style, segment.truecolor)?;
        write_layout_range(out, range, segment.whitespace)
    }
}

struct SegmentOutput<'a> {
    style: Style,
    whitespace: bool,
    truecolor: bool,
    hyperlink: Option<&'a str>,
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
    let mut style_state = StyleState::default();
    style_state.transition(out, style, truecolor)?;
    write!(out, "\x1b[K")?;
    style_state.reset(out)
}

pub(super) fn write_reset<W: Write + ?Sized>(out: &mut W) -> io::Result<()> {
    out.write_all(b"\x1b[0m")
}

pub(super) fn write_styled_text<W: Write + ?Sized>(
    out: &mut W,
    text: &str,
    style: Style,
    truecolor: bool,
) -> io::Result<()> {
    let mut style_state = StyleState::default();
    style_state.transition(out, style, truecolor)?;
    write!(out, "{text}")?;
    style_state.reset(out)
}

pub(super) fn write_styled_padding<W: Write + ?Sized>(
    out: &mut W,
    width: usize,
    style: Style,
    truecolor: bool,
) -> io::Result<()> {
    let mut style_state = StyleState::default();
    style_state.transition(out, style, truecolor)?;
    write!(out, "{:width$}", "", width = width)?;
    style_state.reset(out)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct EmittedStyle {
    fg: Option<Color>,
    bg: Option<Color>,
    bold: bool,
    dim: bool,
    underlined: bool,
    reversed: bool,
    crossed_out: bool,
}

impl From<Style> for EmittedStyle {
    fn from(style: Style) -> Self {
        Self {
            fg: terminal_color(style.fg),
            bg: terminal_color(style.bg),
            bold: style.bold == Some(true),
            dim: style.dim == Some(true),
            underlined: style.underlined == Some(true),
            reversed: style.reversed == Some(true),
            crossed_out: style.crossed_out == Some(true),
        }
    }
}

fn terminal_color(color: Option<Color>) -> Option<Color> {
    match color {
        None | Some(Color::Default) => None,
        color => color,
    }
}

#[derive(Default)]
struct StyleState {
    current: EmittedStyle,
}

impl StyleState {
    fn transition<W: Write + ?Sized>(
        &mut self,
        out: &mut W,
        style: Style,
        truecolor: bool,
    ) -> io::Result<()> {
        let target = EmittedStyle::from(style);
        if target == self.current {
            return Ok(());
        }
        let mut parameters = SgrParameters::new(out);
        write_color_transition(&mut parameters, self.current.fg, target.fg, true, truecolor)?;
        write_color_transition(
            &mut parameters,
            self.current.bg,
            target.bg,
            false,
            truecolor,
        )?;
        write_intensity_transition(&mut parameters, self.current, target)?;
        write_attribute_transition(
            &mut parameters,
            self.current.underlined,
            target.underlined,
            4,
            24,
        )?;
        write_attribute_transition(
            &mut parameters,
            self.current.reversed,
            target.reversed,
            7,
            27,
        )?;
        write_attribute_transition(
            &mut parameters,
            self.current.crossed_out,
            target.crossed_out,
            9,
            29,
        )?;
        parameters.finish()?;
        self.current = target;
        Ok(())
    }

    fn reset<W: Write + ?Sized>(&mut self, out: &mut W) -> io::Result<()> {
        if self.current == EmittedStyle::default() {
            return Ok(());
        }
        self.force_reset(out)
    }

    fn force_reset<W: Write + ?Sized>(&mut self, out: &mut W) -> io::Result<()> {
        write_reset(out)?;
        self.current = EmittedStyle::default();
        Ok(())
    }
}

struct SgrParameters<'a, W: Write + ?Sized> {
    out: &'a mut W,
    started: bool,
}

impl<'a, W: Write + ?Sized> SgrParameters<'a, W> {
    fn new(out: &'a mut W) -> Self {
        Self {
            out,
            started: false,
        }
    }

    fn write(&mut self, parameter: impl Display) -> io::Result<()> {
        self.out
            .write_all(if self.started { b";" } else { b"\x1b[" })?;
        write!(self.out, "{parameter}")?;
        self.started = true;
        Ok(())
    }

    fn finish(self) -> io::Result<()> {
        if self.started {
            self.out.write_all(b"m")?;
        }
        Ok(())
    }
}

fn write_color_transition<W: Write + ?Sized>(
    parameters: &mut SgrParameters<'_, W>,
    current: Option<Color>,
    target: Option<Color>,
    foreground: bool,
    truecolor: bool,
) -> io::Result<()> {
    if current == target {
        return Ok(());
    }
    let Some(color) = target else {
        return parameters.write(if foreground { 39 } else { 49 });
    };
    let layer = if foreground { 38 } else { 48 };
    match color {
        Color::Default => unreachable!("terminal default colors are normalized"),
        Color::Ansi(index) if index < 8 => {
            parameters.write(if foreground { 30 } else { 40 } + u16::from(index))
        }
        Color::Ansi(index) => {
            parameters.write(if foreground { 90 } else { 100 } + u16::from(index - 8))
        }
        Color::Indexed(index) => {
            parameters.write(layer)?;
            parameters.write(5)?;
            parameters.write(index)
        }
        Color::Rgb(red, green, blue) if truecolor => {
            parameters.write(layer)?;
            parameters.write(2)?;
            parameters.write(red)?;
            parameters.write(green)?;
            parameters.write(blue)
        }
        Color::Rgb(red, green, blue) => {
            parameters.write(layer)?;
            parameters.write(5)?;
            parameters.write(crate::config::theme::indexed_fallback(red, green, blue))
        }
    }
}

fn write_intensity_transition<W: Write + ?Sized>(
    parameters: &mut SgrParameters<'_, W>,
    current: EmittedStyle,
    target: EmittedStyle,
) -> io::Result<()> {
    if (current.bold && !target.bold) || (current.dim && !target.dim) {
        parameters.write(22)?;
        if target.bold {
            parameters.write(1)?;
        }
        if target.dim {
            parameters.write(2)?;
        }
        return Ok(());
    }
    if !current.bold && target.bold {
        parameters.write(1)?;
    }
    if !current.dim && target.dim {
        parameters.write(2)?;
    }
    Ok(())
}

fn write_attribute_transition<W: Write + ?Sized>(
    parameters: &mut SgrParameters<'_, W>,
    current: bool,
    target: bool,
    enable: u8,
    disable: u8,
) -> io::Result<()> {
    if current == target {
        return Ok(());
    }
    parameters.write(if target { enable } else { disable })
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
