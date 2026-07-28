//! Purpose: compose an unwrapped visible viewport into one bounded ANSI frame.
//! Owns: visible row fetches, gutters, annotations, and cursor cell positioning.
//! Must not: mutate buffers, flush writers, inspect off-viewport syntax, or own terminal modes.
//! Invariants: every viewport row is cleared; reads stay viewport-bounded; cursor cells are safe.

use std::io;

use crate::buffer::{Buffer, Cursor};

use super::{
    change_gutter_width, line_number_gutter, style, write_external_change_gutter,
    write_line_number, RenderOptions, RenderViewport,
};

pub(super) fn compose_buffer(
    out: &mut Vec<u8>,
    buffer: &dyn Buffer,
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions,
) -> io::Result<()> {
    let RenderViewport {
        start_row,
        start_col,
        height,
        width,
        ..
    } = viewport;
    let content_height = super::content_height(height, options.action_bar);
    let (line_gutter, external_gutter) = gutter_width(buffer, options, width);
    let gutter = line_gutter.saturating_add(external_gutter);
    let content_width = width.saturating_sub(gutter);
    let cursor = buffer.cursor();
    let fetch_width = fetch_width(cursor, start_row, start_col, content_height, content_width);
    let cursor_cells = write_rows(
        out,
        buffer,
        cursor,
        start_row,
        start_col,
        content_height,
        content_width,
        line_gutter,
        external_gutter,
        fetch_width,
        options,
    )?;
    super::write_bottom_rows(out, viewport, message, options)?;
    let position = cursor_position(
        buffer,
        cursor,
        cursor_cells,
        viewport,
        gutter,
        content_height,
    );
    super::emoji_picker::write(
        out,
        position,
        content_height,
        viewport.width,
        options.emoji_picker,
        options.theme,
    )?;
    super::write_terminal_cursor(out, position, options.cursor_shape)
}

fn gutter_width(buffer: &dyn Buffer, options: RenderOptions<'_>, width: usize) -> (usize, usize) {
    let line_gutter = if options.line_numbers {
        line_number_gutter(buffer.line_count())
    } else {
        0
    }
    .min(width);
    let external_gutter = change_gutter_width(
        options
            .external_changes
            .is_some_and(|changes| !changes.markers.is_empty()),
    )
    .min(width.saturating_sub(line_gutter));
    (line_gutter, external_gutter)
}

fn fetch_width(
    cursor: Cursor,
    start_row: usize,
    start_col: usize,
    height: usize,
    content_width: usize,
) -> usize {
    let cursor_window = if cursor.row >= start_row && cursor.row < start_row.saturating_add(height)
    {
        cursor.col.saturating_sub(start_col).saturating_add(1)
    } else {
        0
    };
    content_width
        .saturating_mul(4)
        .saturating_add(32)
        .max(cursor_window)
}

#[allow(clippy::too_many_arguments)]
fn write_rows(
    out: &mut Vec<u8>,
    buffer: &dyn Buffer,
    cursor: Cursor,
    start_row: usize,
    start_col: usize,
    height: usize,
    width: usize,
    line_gutter: usize,
    external_gutter: usize,
    fetch_width: usize,
    options: RenderOptions<'_>,
) -> io::Result<usize> {
    let mut layout = crate::editor::text_layout::VisibleLineLayout::default();
    let mut boundaries = Vec::new();
    let mut cursor_cells = 0;
    for screen_row in 1..=height {
        style::write_row_start(out, screen_row, options.theme.text, options.theme.truecolor)?;
        if external_gutter > 0 {
            write_external_change_gutter(
                out,
                start_row + screen_row - 1,
                options.external_changes,
                options.theme,
            )?;
        }
        if line_gutter > 0 {
            write_line_number(out, start_row + screen_row - 1, line_gutter, options.theme)?;
        }
        if width > 0 {
            let document_row = start_row + screen_row - 1;
            if document_row < buffer.line_count() {
                let content = super::boundary_complete_line(
                    buffer,
                    document_row,
                    start_col,
                    fetch_width,
                    width,
                    false,
                    &mut layout,
                )?;
                style::write_content_line_from_layout(
                    out,
                    &content,
                    document_row,
                    start_col,
                    options,
                    &layout,
                    &mut boundaries,
                )?;
                if cursor.row == document_row && cursor.col >= start_col {
                    cursor_cells = layout.scalar_to_cell(cursor.col.saturating_sub(start_col));
                }
            }
        }
        style::write_reset(out)?;
    }
    Ok(cursor_cells)
}

fn cursor_position(
    buffer: &dyn Buffer,
    cursor: Cursor,
    cursor_cells: usize,
    viewport: RenderViewport,
    gutter: usize,
    content_height: usize,
) -> Option<(usize, usize)> {
    let content_width = viewport.width.saturating_sub(gutter);
    let Cursor { row, col } = cursor;
    let row_visible =
        row >= viewport.start_row && row < viewport.start_row.saturating_add(content_height);
    let line_end = buffer.line_char_count(row).unwrap_or(0);
    let col_visible = col >= viewport.start_col
        && (cursor_cells < content_width || (col == line_end && cursor_cells == content_width));
    (row_visible && col_visible && content_width > 0).then(|| {
        (
            row - viewport.start_row + 1,
            gutter
                .saturating_add(cursor_cells)
                .saturating_add(1)
                .min(viewport.width.max(1)),
        )
    })
}
