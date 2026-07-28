//! Purpose: transport complete, bounded in-memory render frames to the terminal writer.
//! Owns: render input types, synchronized frame envelopes, one frame write, and one flush.
//! Must not: mutate editor/buffer state, read full buffers, or own terminal setup.
//! Invariants: composition errors produce no output; every update hides the cursor and is published
//!   as one synchronized frame with explicit dimension/work bounds.

use std::borrow::Cow;
use std::io::{self, Write};

use crate::buffer::{Buffer, Cursor};
use crate::config::theme::Theme;
use crate::editor::markdown_preview::MarkdownAnnotations;
use crate::editor::syntax::SyntaxKind;
use crate::editor::text_layout::VisibleLineLayout;
use crate::terminal::cursor_style::{self, CursorShape};

#[cfg(test)]
mod coherence_tests;
#[cfg(test)]
mod cursor_tests;
mod emoji_picker;
mod frame;
mod status_bar;
mod style;
pub(crate) mod wrapped;

pub(crate) use status_bar::{StatusRole, StatusTheme};

const MAX_FRAME_DIMENSION: usize = 16_384;
const MAX_FRAME_CELLS: usize = 8 * 1024 * 1024;
pub(crate) const SYNC_UPDATE_BEGIN: &[u8] = b"\x1b[?2026h";
pub(crate) const SYNC_UPDATE_END: &[u8] = b"\x1b[?2026l";
/// Abort one partial string command, close OSC 8, then restore default SGR.
///
/// The order is intentional: no CSI/synchronized-update command is emitted
/// until a truncated OSC or DCS from an earlier transport failure is closed.
pub(crate) const TERMINAL_STATE_RECOVERY: &[u8] = b"\x1b\\\x1b]8;;\x1b\\\x1b[0m";
const HIDE_CURSOR: &[u8] = b"\x1b[?25l";

/// Fetch enough scalars to prove that the retained layout ends at a complete
/// grapheme boundary. A trailing partial cluster is ambiguous only when layout
/// consumed the whole fetched string while the logical line still has text.
pub(super) fn boundary_complete_line<'a>(
    buffer: &'a dyn Buffer,
    row: usize,
    start_col: usize,
    initial_fetch: usize,
    max_cells: usize,
    wrapped: bool,
    layout: &mut VisibleLineLayout,
) -> io::Result<Cow<'a, str>> {
    let line_len = buffer.line_char_count(row).unwrap_or(0);
    let remaining = line_len.saturating_sub(start_col);
    if remaining == 0 {
        layout.build("", max_cells);
        return Ok(Cow::Borrowed(""));
    }
    let mut fetch = initial_fetch.max(1).min(remaining);
    loop {
        let content = buffer
            .try_visible_lines_window(row, 1, start_col, fetch)?
            .into_iter()
            .next()
            .map(|line| line.content)
            .unwrap_or_default();
        if wrapped {
            layout.build_wrapped(&content, max_cells);
        } else {
            layout.build(&content, max_cells);
        }
        let needs_boundary_completion = layout.source_byte_len() == content.len()
            && start_col.saturating_add(layout.source_scalar_len()) < line_len;
        if !needs_boundary_completion {
            return Ok(content);
        }
        #[cfg(test)]
        crate::editor::text_layout::record_visible_layout_probe();
        let next_fetch = fetch
            .saturating_mul(2)
            .max(fetch.saturating_add(1))
            .min(remaining);
        if next_fetch == fetch {
            return Ok(content);
        }
        fetch = next_fetch;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextHighlight {
    pub(crate) start: Cursor,
    pub(crate) end: Cursor,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum HighlightKind {
    #[default]
    Selection,
    Search,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ContentSurface {
    #[default]
    Normal,
    Preview,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExternalChangeKind {
    Added,
    Changed,
    Deleted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExternalLineMarker {
    pub(crate) line: usize,
    pub(crate) kind: ExternalChangeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ExternalChanges<'a> {
    pub(crate) added_ranges: &'a [TextHighlight],
    pub(crate) changed_ranges: &'a [TextHighlight],
    pub(crate) markers: &'a [ExternalLineMarker],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DocumentPresentation<'a> {
    pub(crate) annotations: &'a MarkdownAnnotations,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EmojiPicker<'a> {
    pub(crate) rows: &'a [String],
    pub(crate) selected: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RenderOptions<'a> {
    pub(crate) cursor_shape: CursorShape,
    pub(crate) highlight: Option<TextHighlight>,
    pub(crate) highlight_kind: HighlightKind,
    pub(crate) lint_ranges: Option<&'a [TextHighlight]>,
    pub(crate) external_changes: Option<ExternalChanges<'a>>,
    pub(crate) syntax: SyntaxKind,
    pub(crate) presentation: Option<DocumentPresentation<'a>>,
    pub(crate) surface: ContentSurface,
    pub(crate) theme: Theme,
    pub(crate) line_numbers: bool,
    pub(crate) whitespace: bool,
    pub(crate) soft_wrap: bool,
    pub(crate) status_role: StatusRole,
    pub(crate) status_theme: StatusTheme,
    pub(crate) status_path: Option<(usize, usize)>,
    pub(crate) status_filename: Option<(usize, usize)>,
    pub(crate) status_selection: Option<(usize, usize)>,
    pub(crate) emoji_picker: Option<EmojiPicker<'a>>,
    pub(crate) window_title: Option<&'a str>,
    /// Optional second bottom row for touch actions.
    pub(crate) action_bar: Option<&'a str>,
}

impl Default for RenderOptions<'_> {
    fn default() -> Self {
        Self {
            cursor_shape: CursorShape::Default,
            highlight: None,
            highlight_kind: HighlightKind::Selection,
            lint_ranges: None,
            external_changes: None,
            syntax: SyntaxKind::Plain,
            presentation: None,
            surface: ContentSurface::Normal,
            theme: Theme::default(),
            line_numbers: false,
            whitespace: false,
            soft_wrap: false,
            status_role: StatusRole::Normal,
            status_theme: StatusTheme::default(),
            status_path: None,
            status_filename: None,
            status_selection: None,
            emoji_picker: None,
            window_title: None,
            action_bar: None,
        }
    }
}

pub(super) fn write_terminal_cursor(
    out: &mut Vec<u8>,
    position: Option<(usize, usize)>,
    shape: CursorShape,
) -> io::Result<()> {
    cursor_style::write_shape(out, shape)?;
    match position {
        Some((row, col)) => write!(out, "\x1b[{row};{col}H\x1b[?25h"),
        None => write!(out, "\x1b[?25l\x1b[1;1H"),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderViewport {
    start_row: usize,
    start_col: usize,
    height: usize,
    width: usize,
    wrap_col: usize,
}

impl RenderViewport {
    pub const fn new(start_row: usize, start_col: usize, height: usize, width: usize) -> Self {
        Self {
            start_row,
            start_col,
            height,
            width,
            wrap_col: 0,
        }
    }

    pub(crate) const fn with_wrap_col(mut self, wrap_col: usize) -> Self {
        self.wrap_col = wrap_col;
        self
    }
}

pub(crate) fn line_number_gutter(line_count: usize) -> usize {
    line_count.max(1).to_string().len().saturating_add(1)
}

pub(crate) fn change_gutter_width(has_changes: bool) -> usize {
    usize::from(has_changes) * 2
}

pub(super) fn content_height(height: usize, action_bar: Option<&str>) -> usize {
    super::screen::bottom_layout(height, action_bar.is_some()).content_height
}

pub(super) fn write_bottom_rows(
    out: &mut Vec<u8>,
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions<'_>,
) -> io::Result<()> {
    let layout = super::screen::bottom_layout(viewport.height, options.action_bar.is_some());
    if let Some(separator_row) = layout.separator_row {
        write!(out, "\x1b[{separator_row};1H\x1b[0m\x1b[2K")?;
    }
    if let Some(status_row) = layout.status_row {
        status_bar::write_status_bar(
            out,
            status_row,
            viewport.width,
            message.unwrap_or(""),
            status_bar::StatusBarPresentation {
                role: options.status_role,
                theme: options.status_theme,
                path: options.status_path,
                filename: options.status_filename,
                selection: options.status_selection,
            },
        )?;
    }
    if let Some((action_row, action_bar)) = layout.action_row.zip(options.action_bar) {
        status_bar::write_status_bar(
            out,
            action_row,
            viewport.width,
            action_bar,
            status_bar::StatusBarPresentation {
                role: StatusRole::Info,
                theme: options.status_theme,
                path: None,
                filename: None,
                selection: None,
            },
        )?;
    }
    Ok(())
}

/// Basic viewport render with one optional active search highlight.
/// Clears each viewport row, writes the visible window using visible_lines
/// (not the full .lines() clone), positions the terminal cursor exactly at
/// the buffer's logical cursor. No phantom line is appended after the last
/// rendered row.
///
/// `viewport` defines the visible row/column origin and terminal dimensions.
/// Bottom row (height) reserved for minimal message if provided; content uses height-1.
/// Horizontal slicing starts at a scalar document column but clips by terminal cells.
/// Least invasive addition: message shown on last row via absolute positioning.
pub fn render_buffer<W: Write + ?Sized>(
    out: &mut W,
    buffer: &dyn Buffer,
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions<'_>,
) -> io::Result<()> {
    validate_frame_size(viewport)?;
    let mut frame = Vec::new();
    begin_frame(&mut frame)?;
    super::title::write(&mut frame, options.window_title)?;
    style::write_cursor_color(&mut frame, options.theme)?;
    if options.soft_wrap {
        wrapped::compose_buffer(&mut frame, buffer, viewport, message, options)?;
    } else {
        frame::compose_buffer(&mut frame, buffer, viewport, message, options)?;
    }
    end_frame(&mut frame)?;
    out.write_all(&frame)?;
    out.flush()
}

fn begin_frame(frame: &mut Vec<u8>) -> io::Result<()> {
    frame.write_all(TERMINAL_STATE_RECOVERY)?;
    frame.write_all(SYNC_UPDATE_BEGIN)?;
    frame.write_all(HIDE_CURSOR)
}

fn end_frame(frame: &mut Vec<u8>) -> io::Result<()> {
    frame.write_all(SYNC_UPDATE_END)
}

fn validate_frame_size(viewport: RenderViewport) -> io::Result<()> {
    let within_dimensions =
        viewport.height <= MAX_FRAME_DIMENSION && viewport.width <= MAX_FRAME_DIMENSION;
    let within_cells = viewport
        .height
        .checked_mul(viewport.width)
        .is_some_and(|cells| cells <= MAX_FRAME_CELLS);
    if within_dimensions && within_cells {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "terminal dimensions exceed the bounded render-frame limit",
        ))
    }
}

pub(super) fn write_line_number<W: Write + ?Sized>(
    out: &mut W,
    row: usize,
    gutter: usize,
    theme: Theme,
) -> std::io::Result<()> {
    let label = format!(
        "{:>width$} ",
        row.saturating_add(1),
        width = gutter.saturating_sub(1)
    );
    let clipped: String = label.chars().take(gutter).collect();
    style::write_styled_text(
        out,
        &clipped,
        theme.text.overlay(theme.line_number),
        theme.truecolor,
    )
}

pub(super) fn write_external_change_gutter<W: Write + ?Sized>(
    out: &mut W,
    row: usize,
    changes: Option<ExternalChanges<'_>>,
    theme: Theme,
) -> std::io::Result<()> {
    let Some(changes) = changes else {
        return write!(out, "  ");
    };
    let Ok(marker) = changes
        .markers
        .binary_search_by_key(&row, |marker| marker.line)
    else {
        return write!(out, "  ");
    };
    let marker = &changes.markers[marker];
    let (symbol, style) = match marker.kind {
        ExternalChangeKind::Added => ("+", theme.external_added),
        ExternalChangeKind::Changed => ("~", theme.external_changed),
        ExternalChangeKind::Deleted => ("-", theme.external_deleted),
    };
    style::write_styled_text(
        out,
        symbol,
        style.overlay(crate::config::theme::Style {
            bold: Some(true),
            ..crate::config::theme::Style::default()
        }),
        theme.truecolor,
    )?;
    write!(out, " ")
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod transport_tests;
