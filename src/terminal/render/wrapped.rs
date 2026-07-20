//! Purpose: render and map bounded viewport rows for explicit soft wrapping.
//! Owns: logical-to-visual row splitting, continuation gutters, and wrapped cursor placement.
//! Must not: mutate buffers/App state, scan whole files, own terminal setup, or save.
//! Invariants: no whole-document scan; forward work is viewport-bounded; visual slices end on
//!   grapheme boundaries, and each reverse step inspects at most one logical line.
//! Phase: post-v0.1 core usability.

use std::io::{self, Write};

use crate::buffer::{Buffer, Cursor};
use crate::editor::text_layout;

use super::{
    change_gutter_width, line_number_gutter, write_change_gutter, write_external_change_gutter,
    write_line_number, RenderOptions, RenderViewport,
};

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
    let rows = visible_rows(buffer, start_row, wrap_col, height, width)?;
    Ok(wrapped_cursor_position(cursor, &rows, 0, width).is_some())
}

pub(crate) fn scroll_origin(
    buffer: &dyn Buffer,
    start_row: usize,
    wrap_col: usize,
    height: usize,
    width: usize,
    count: usize,
    forward: bool,
) -> io::Result<(usize, usize)> {
    if width == 0 || height == 0 || count == 0 {
        return Ok((start_row, wrap_col));
    }
    if forward {
        scroll_origin_forward(buffer, start_row, wrap_col, height, width, count)
    } else {
        scroll_origin_backward(buffer, start_row, wrap_col, width, count)
    }
}

fn scroll_origin_forward(
    buffer: &dyn Buffer,
    start_row: usize,
    wrap_col: usize,
    height: usize,
    width: usize,
    count: usize,
) -> io::Result<(usize, usize)> {
    let rows = visible_rows(
        buffer,
        start_row,
        wrap_col,
        height.saturating_add(count),
        width,
    )?;
    let advance = count.min(rows.len().saturating_sub(height));
    Ok(rows
        .get(advance)
        .map(|row| (row.document_row, row.start_col))
        .unwrap_or((start_row, wrap_col)))
}

fn scroll_origin_backward(
    buffer: &dyn Buffer,
    mut row: usize,
    mut col: usize,
    width: usize,
    count: usize,
) -> io::Result<(usize, usize)> {
    for _ in 0..count {
        let previous = previous_origin(buffer, row, col, width)?;
        if previous == (row, col) {
            break;
        }
        (row, col) = previous;
    }
    Ok((row, col))
}

fn previous_origin(
    buffer: &dyn Buffer,
    row: usize,
    col: usize,
    width: usize,
) -> io::Result<(usize, usize)> {
    if row == 0 && col == 0 {
        return Ok((0, 0));
    }
    let (target_row, target_col) = if col == 0 {
        let target_row = row.saturating_sub(1);
        (target_row, buffer.line_char_count(target_row).unwrap_or(0))
    } else {
        (row, col)
    };
    Ok((
        target_row,
        wrapped_start_before(buffer, target_row, target_col, width)?,
    ))
}

fn wrapped_start_before(
    buffer: &dyn Buffer,
    row: usize,
    end_col: usize,
    width: usize,
) -> io::Result<usize> {
    let mut start_col = 0;
    while start_col < end_col {
        let mut row_slice = Vec::with_capacity(1);
        append_line_rows(buffer, row, start_col, 1, width, &mut row_slice)?;
        let Some(next_col) = row_slice.first().map(WrappedRow::end_col) else {
            break;
        };
        if next_col >= end_col || next_col <= start_col {
            break;
        }
        start_col = next_col;
    }
    Ok(start_col.min(end_col))
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
    options: RenderOptions<'_>,
) -> io::Result<()> {
    let content_height = super::content_height(viewport.height, options.action_bar);
    let line_gutter = if options.line_numbers {
        line_number_gutter(buffer.line_count())
    } else {
        0
    }
    .min(viewport.width);
    let external_gutter = change_gutter_width(
        options
            .external_changes
            .is_some_and(|changes| !changes.markers.is_empty()),
    )
    .min(viewport.width.saturating_sub(line_gutter));
    let llm_gutter = change_gutter_width(
        options
            .llm_changes
            .is_some_and(|changes| !changes.gutter_lines.is_empty()),
    )
    .min(
        viewport
            .width
            .saturating_sub(line_gutter)
            .saturating_sub(external_gutter),
    );
    let gutter = line_gutter
        .saturating_add(external_gutter)
        .saturating_add(llm_gutter);
    let content_width = viewport.width.saturating_sub(gutter);
    let rows = visible_rows(
        buffer,
        viewport.start_row,
        viewport.wrap_col,
        content_height,
        content_width,
    )?;
    write_rows(
        out,
        &rows,
        content_height,
        content_width,
        (line_gutter, external_gutter, llm_gutter),
        options,
    )?;
    super::write_bottom_rows(out, viewport, message, options)?;
    let cursor = wrapped_cursor_position(buffer.cursor(), &rows, gutter, content_width);
    super::write_terminal_cursor(out, cursor, options.cursor_shape)
}

pub(super) fn append_line_rows(
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
    gutters: (usize, usize, usize),
    options: RenderOptions<'_>,
) -> io::Result<()> {
    let (line_gutter, external_gutter, llm_gutter) = gutters;
    for screen_row in 1..=height {
        super::style::write_row_start(
            out,
            screen_row,
            options.theme.text,
            options.theme.truecolor,
        )?;
        let Some(row) = rows.get(screen_row - 1) else {
            continue;
        };
        if external_gutter > 0 && row.start_col == 0 {
            write_external_change_gutter(
                out,
                row.document_row,
                options.external_changes,
                options.theme,
            )?;
        } else if external_gutter > 0 {
            write!(out, "{:external_gutter$}", "")?;
        }
        if llm_gutter > 0 && row.start_col == 0 {
            write_change_gutter(out, row.document_row, options.llm_changes, options.theme)?;
        } else if llm_gutter > 0 {
            write!(out, "{:llm_gutter$}", "")?;
        }
        if line_gutter > 0 && row.start_col == 0 {
            write_line_number(out, row.document_row, line_gutter, options.theme)?;
        } else if line_gutter > 0 {
            let blank = " ".repeat(line_gutter);
            super::style::write_styled_text(
                out,
                &blank,
                options.theme.text.overlay(options.theme.line_number),
                options.theme.truecolor,
            )?;
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

fn wrapped_cursor_position(
    cursor: Cursor,
    rows: &[WrappedRow],
    gutter: usize,
    width: usize,
) -> Option<(usize, usize)> {
    let (index, row) = rows
        .iter()
        .enumerate()
        .find(|(_, row)| row_contains_cursor(row, cursor))?;
    let cell = text_layout::scalar_to_cell(&row.content, cursor.col.saturating_sub(row.start_col));
    if width == 0 || cell > width || (cell == width && !row.line_end) {
        return None;
    }
    Some((
        index.saturating_add(1),
        gutter
            .saturating_add(cell)
            .saturating_add(1)
            .min(gutter.saturating_add(width).max(1)),
    ))
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
