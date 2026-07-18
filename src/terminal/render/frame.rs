//! Purpose: compose an unwrapped visible viewport into one bounded ANSI frame.
//! Owns: visible row fetches, gutters, annotations, and cursor cell positioning.
//! Must not: mutate buffers, flush writers, inspect off-viewport syntax, or own terminal modes.
//! Invariants: every viewport row is cleared; reads stay viewport-bounded; cursor cells are safe.
//! Phase: issue #62 semantic theme integration.

use std::io;

use crate::buffer::{Buffer, Cursor, LineView};

use super::{
    change_gutter_width, line_number_gutter, style, write_change_gutter, write_line_number,
    RenderOptions, RenderViewport,
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
    let content_height = height.saturating_sub(1);
    let (line_gutter, change_gutter) = gutter_width(buffer, options, width);
    let gutter = line_gutter.saturating_add(change_gutter);
    let content_width = width.saturating_sub(gutter);
    let cursor = buffer.cursor();
    let fetch_width = fetch_width(cursor, start_row, start_col, content_height, content_width);
    let visible =
        buffer.try_visible_lines_window(start_row, content_height, start_col, fetch_width)?;
    write_rows(
        out,
        &visible,
        start_row,
        start_col,
        content_height,
        content_width,
        line_gutter,
        change_gutter,
        options,
    )?;
    if height > 0 {
        super::status_bar::write_status_bar(
            out,
            height,
            width,
            message.unwrap_or(""),
            options.status_role,
            options.status_theme,
        )?;
    }
    let position = cursor_position(buffer, cursor, &visible, viewport, gutter);
    super::write_terminal_cursor(out, position, options.cursor_shape)
}

fn gutter_width(buffer: &dyn Buffer, options: RenderOptions<'_>, width: usize) -> (usize, usize) {
    let line_gutter = if options.line_numbers {
        line_number_gutter(buffer.line_count())
    } else {
        0
    }
    .min(width);
    let change_gutter = change_gutter_width(
        options
            .llm_changes
            .is_some_and(|changes| !changes.gutter_lines.is_empty()),
    )
    .min(width.saturating_sub(line_gutter));
    (line_gutter, change_gutter)
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
    visible: &[LineView],
    start_row: usize,
    start_col: usize,
    height: usize,
    width: usize,
    line_gutter: usize,
    change_gutter: usize,
    options: RenderOptions<'_>,
) -> io::Result<()> {
    for screen_row in 1..=height {
        style::write_row_start(out, screen_row, options.theme.text, options.theme.truecolor)?;
        if change_gutter > 0 {
            write_change_gutter(
                out,
                start_row + screen_row - 1,
                options.llm_changes,
                options.theme,
            )?;
        }
        if line_gutter > 0 {
            write_line_number(out, start_row + screen_row - 1, line_gutter, options.theme)?;
        }
        if width > 0 {
            if let Some(line) = visible.get(screen_row - 1) {
                style::write_content_line(
                    out,
                    &line.content,
                    start_row + screen_row - 1,
                    start_col,
                    width,
                    options,
                )?;
            }
        }
    }
    Ok(())
}

fn cursor_position(
    buffer: &dyn Buffer,
    cursor: Cursor,
    visible: &[LineView],
    viewport: RenderViewport,
    gutter: usize,
) -> Option<(usize, usize)> {
    let content_height = viewport.height.saturating_sub(1);
    let content_width = viewport.width.saturating_sub(gutter);
    let Cursor { row, col } = cursor;
    let row_visible =
        row >= viewport.start_row && row < viewport.start_row.saturating_add(content_height);
    let cells = if row_visible && col >= viewport.start_col {
        visible
            .get(row - viewport.start_row)
            .map(|line| {
                crate::editor::text_layout::scalar_to_cell(
                    &line.content,
                    col.saturating_sub(viewport.start_col),
                )
            })
            .unwrap_or(0)
    } else {
        0
    };
    let line_end = buffer.line_char_count(row).unwrap_or(0);
    let col_visible = col >= viewport.start_col
        && (cells < content_width || (col == line_end && cells == content_width));
    (row_visible && col_visible && content_width > 0).then(|| {
        (
            row - viewport.start_row + 1,
            gutter
                .saturating_add(cells)
                .saturating_add(1)
                .min(viewport.width.max(1)),
        )
    })
}
