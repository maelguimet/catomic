//! Purpose: render and map bounded viewport rows for explicit soft wrapping.
//! Owns: logical-to-visual row splitting, continuation gutters, and wrapped cursor placement.
//! Must not: mutate buffers/App state, scan whole files, own terminal setup, or save.
//! Invariants: work is bounded by viewport rows; visual slices end on grapheme boundaries.
//! Phase: post-v0.1 core usability.

use std::io::{self, Write};

use crate::buffer::{Buffer, Cursor};
use crate::editor::text_layout;

use super::{line_number_gutter, write_line_number, RenderOptions, RenderViewport};

#[derive(Clone, Debug)]
pub(crate) struct WrappedRow {
    pub(crate) document_row: usize,
    pub(crate) start_col: usize,
    pub(crate) content: String,
    pub(crate) line_end: bool,
}

impl WrappedRow {
    pub(crate) fn end_col(&self) -> usize {
        self.start_col.saturating_add(self.content.chars().count())
    }
}

pub(crate) fn visible_rows(
    buffer: &dyn Buffer,
    start_row: usize,
    wrap_col: usize,
    height: usize,
    width: usize,
) -> io::Result<Vec<WrappedRow>> {
    let mut rows = Vec::with_capacity(height);
    let mut document_row = start_row;
    let mut start_col = wrap_col;
    while rows.len() < height && document_row < buffer.line_count() {
        append_line_rows(
            buffer,
            document_row,
            start_col,
            height - rows.len(),
            width,
            &mut rows,
        )?;
        document_row = document_row.saturating_add(1);
        start_col = 0;
    }
    Ok(rows)
}

pub(crate) fn cursor_is_visible(
    buffer: &dyn Buffer,
    start_row: usize,
    wrap_col: usize,
    height: usize,
    width: usize,
) -> io::Result<bool> {
    let cursor = buffer.cursor();
    Ok(visible_rows(buffer, start_row, wrap_col, height, width)?
        .iter()
        .any(|row| row_contains_cursor(row, cursor)))
}

pub(crate) fn start_col_near_cursor(
    buffer: &dyn Buffer,
    cursor: Cursor,
    height: usize,
    width: usize,
) -> io::Result<usize> {
    if width == 0 || height == 0 {
        return Ok(0);
    }
    let capacity = width.saturating_mul(height).saturating_sub(1);
    let approximate = cursor.col.saturating_sub(capacity);
    let context_start = approximate.saturating_sub(64);
    let fetch_width = cursor.col.saturating_sub(context_start).saturating_add(1);
    let text = line_window(buffer, cursor.row, context_start, fetch_width)?;
    let requested = approximate.saturating_sub(context_start);
    let boundary = text_layout::ceil_to_grapheme_col(&text, requested);
    let visible: String = text.chars().skip(boundary).collect();
    let cursor_col = cursor
        .col
        .saturating_sub(context_start.saturating_add(boundary));
    let cursor_cell = text_layout::scalar_to_cell(&visible, cursor_col);
    let hidden = cursor_cell.saturating_sub(capacity);
    Ok(context_start
        .saturating_add(boundary)
        .saturating_add(text_layout::scalar_at_cell(&visible, hidden)))
}

pub(super) fn compose_buffer(
    out: &mut Vec<u8>,
    buffer: &dyn Buffer,
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions,
) -> io::Result<()> {
    let content_height = viewport.height.saturating_sub(1);
    let gutter = if options.line_numbers {
        line_number_gutter(buffer.line_count())
    } else {
        0
    }
    .min(viewport.width);
    let content_width = viewport.width.saturating_sub(gutter);
    let rows = visible_rows(
        buffer,
        viewport.start_row,
        viewport.wrap_col,
        content_height,
        content_width,
    )?;
    write_rows(out, &rows, content_height, content_width, gutter, options)?;
    if viewport.height > 0 {
        super::status_bar::write_status_bar(
            out,
            viewport.height,
            viewport.width,
            message.unwrap_or(""),
            options.status_role,
            options.status_theme,
        )?;
    }
    let (cursor_row, cursor_col) = wrapped_cursor_position(buffer.cursor(), &rows, gutter);
    crate::terminal::cursor_style::write_shape(out, options.cursor_shape)?;
    write!(out, "\x1b[{cursor_row};{cursor_col}H")
}

fn append_line_rows(
    buffer: &dyn Buffer,
    document_row: usize,
    mut start_col: usize,
    limit: usize,
    width: usize,
    rows: &mut Vec<WrappedRow>,
) -> io::Result<()> {
    let line_len = buffer.line_char_count(document_row).unwrap_or(0);
    if start_col >= line_len || width == 0 {
        rows.push(WrappedRow {
            document_row,
            start_col: start_col.min(line_len),
            content: String::new(),
            line_end: true,
        });
        return Ok(());
    }
    for _ in 0..limit {
        let fetch = width.saturating_mul(4).saturating_add(32);
        let text = line_window(buffer, document_row, start_col, fetch)?;
        let mut take = text_layout::clipped_scalar_len(&text, width);
        if take == 0 {
            take = text_layout::next_grapheme_col(&text, 0);
        }
        let content: String = text.chars().take(take).collect();
        let end_col = start_col.saturating_add(content.chars().count());
        let line_end = end_col >= line_len;
        rows.push(WrappedRow {
            document_row,
            start_col,
            content,
            line_end,
        });
        start_col = end_col;
        if line_end {
            break;
        }
    }
    Ok(())
}

fn write_rows<W: Write + ?Sized>(
    out: &mut W,
    rows: &[WrappedRow],
    height: usize,
    width: usize,
    gutter: usize,
    options: RenderOptions,
) -> io::Result<()> {
    for screen_row in 1..=height {
        write!(out, "\x1b[{screen_row};1H\x1b[K")?;
        let Some(row) = rows.get(screen_row - 1) else {
            continue;
        };
        if gutter > 0 && row.start_col == 0 {
            write_line_number(out, row.document_row, gutter)?;
        } else if gutter > 0 {
            write!(out, "{:gutter$}", "")?;
        }
        super::style::write_content_line(
            out,
            &row.content,
            row.document_row,
            row.start_col,
            width.max(1),
            options,
        )?;
    }
    Ok(())
}

fn wrapped_cursor_position(cursor: Cursor, rows: &[WrappedRow], gutter: usize) -> (usize, usize) {
    let Some((index, row)) = rows
        .iter()
        .enumerate()
        .find(|(_, row)| row_contains_cursor(row, cursor))
    else {
        return (1, gutter.saturating_add(1));
    };
    let cell = text_layout::scalar_to_cell(&row.content, cursor.col.saturating_sub(row.start_col));
    (
        index.saturating_add(1),
        gutter.saturating_add(cell).saturating_add(1),
    )
}

fn row_contains_cursor(row: &WrappedRow, cursor: Cursor) -> bool {
    cursor.row == row.document_row
        && cursor.col >= row.start_col
        && (cursor.col < row.end_col() || (row.line_end && cursor.col == row.end_col()))
}

fn line_window(
    buffer: &dyn Buffer,
    row: usize,
    start_col: usize,
    width: usize,
) -> io::Result<String> {
    Ok(buffer
        .try_visible_lines_window(row, 1, start_col, width)?
        .into_iter()
        .next()
        .map(|line| line.content)
        .unwrap_or_default())
}

#[cfg(test)]
mod tests;
