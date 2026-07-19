//! Purpose: transport complete, bounded in-memory render frames to the terminal writer.
//! Owns: render input types, composer selection, one frame write, and one flush.
//! Must not: mutate editor/buffer state, read full buffers, or own terminal setup.
//! Invariants: composition errors produce no output; frames have explicit dimension/work bounds.
//! Phase: bounded post-beta render ownership cleanup.

use std::io::{self, Write};

use crate::buffer::{Buffer, Cursor};
use crate::config::theme::Theme;
use crate::editor::syntax::SyntaxKind;
use crate::terminal::cursor_style::{self, CursorShape};

#[cfg(test)]
mod cursor_tests;
mod frame;
mod ghost;
mod status_bar;
mod style;
pub(crate) mod wrapped;

pub(crate) use status_bar::{StatusRole, StatusTheme};

const MAX_FRAME_DIMENSION: usize = 16_384;
const MAX_FRAME_CELLS: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextHighlight {
    pub(crate) start: Cursor,
    pub(crate) end: Cursor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GhostText<'a> {
    pub(crate) cursor: Cursor,
    pub(crate) text: &'a str,
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
    Diff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LlmChanges<'a> {
    pub(crate) ranges: &'a [TextHighlight],
    pub(crate) gutter_lines: &'a [usize],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RenderOptions<'a> {
    pub(crate) cursor_shape: CursorShape,
    pub(crate) highlight: Option<TextHighlight>,
    pub(crate) highlight_kind: HighlightKind,
    pub(crate) llm_changes: Option<LlmChanges<'a>>,
    pub(crate) syntax: SyntaxKind,
    pub(crate) surface: ContentSurface,
    pub(crate) theme: Theme,
    pub(crate) line_numbers: bool,
    pub(crate) whitespace: bool,
    pub(crate) soft_wrap: bool,
    pub(crate) status_role: StatusRole,
    pub(crate) status_theme: StatusTheme,
    pub(crate) status_filename: Option<(usize, usize)>,
    pub(crate) status_selection: Option<(usize, usize)>,
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
            llm_changes: None,
            syntax: SyntaxKind::Plain,
            surface: ContentSurface::Normal,
            theme: Theme::default(),
            line_numbers: false,
            whitespace: false,
            soft_wrap: false,
            status_role: StatusRole::Normal,
            status_theme: StatusTheme::default(),
            status_filename: None,
            status_selection: None,
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
    height
        .saturating_sub(1)
        .saturating_sub(usize::from(action_bar.is_some()))
}

pub(super) fn write_bottom_rows(
    out: &mut Vec<u8>,
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions<'_>,
) -> io::Result<()> {
    let action_rows = usize::from(options.action_bar.is_some());
    let status_row = viewport.height.saturating_sub(action_rows);
    if status_row > 0 {
        status_bar::write_status_bar(
            out,
            status_row,
            viewport.width,
            message.unwrap_or(""),
            options.status_role,
            options.status_theme,
            options.status_filename,
            options.status_selection,
        )?;
    }
    if let Some(action_bar) = options.action_bar.filter(|_| viewport.height > 0) {
        status_bar::write_status_bar(
            out,
            viewport.height,
            viewport.width,
            action_bar,
            StatusRole::Info,
            options.status_theme,
            None,
            None,
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
    super::title::write(&mut frame, options.window_title)?;
    style::write_cursor_color(&mut frame, options.theme)?;
    if options.soft_wrap {
        wrapped::compose_buffer(&mut frame, buffer, viewport, message, options)?;
    } else {
        frame::compose_buffer(&mut frame, buffer, viewport, message, options)?;
    }
    out.write_all(&frame)?;
    out.flush()
}

pub(crate) fn render_buffer_with_ghost<W: Write + ?Sized>(
    out: &mut W,
    buffer: &dyn Buffer,
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions<'_>,
    ghost: Option<GhostText<'_>>,
) -> io::Result<()> {
    let Some(ghost) = ghost.filter(|ghost| ghost.cursor == buffer.cursor()) else {
        return render_buffer(out, buffer, viewport, message, options);
    };
    validate_frame_size(viewport)?;
    let mut frame = Vec::new();
    super::title::write(&mut frame, options.window_title)?;
    style::write_cursor_color(&mut frame, options.theme)?;
    ghost::compose_buffer(&mut frame, buffer, viewport, message, options, ghost)?;
    out.write_all(&frame)?;
    out.flush()
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

pub(super) fn write_change_gutter<W: Write + ?Sized>(
    out: &mut W,
    row: usize,
    changes: Option<LlmChanges<'_>>,
    theme: Theme,
) -> std::io::Result<()> {
    let Some(changes) = changes else {
        return write!(out, "  ");
    };
    if changes.gutter_lines.contains(&row) {
        style::write_semantic_gutter(out, theme.llm_changed, theme.truecolor)
    } else {
        write!(out, "  ")
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod transport_tests;
