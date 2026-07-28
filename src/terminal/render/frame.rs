//! Purpose: compose an unwrapped visible viewport into one bounded ANSI frame.
//! Owns: visible row fetches, gutters, annotations, and cursor cell positioning.
//! Must not: mutate buffers, flush writers, inspect off-viewport syntax, or own terminal modes.
//! Invariants: every viewport row is cleared; reads stay viewport-bounded; cursor cells are safe.

use std::borrow::Cow;
use std::io;

use crate::buffer::{Buffer, Cursor, LineView};
use crate::editor::text_layout::VisibleLineLayout;

use super::{
    change_gutter_width, line_number_gutter, style, write_external_change_gutter,
    write_line_number, RenderOptions, RenderViewport,
};

struct PlannedRow {
    document_row: usize,
    start_col: usize,
    content_fingerprint: Option<u64>,
    layout: VisibleLineLayout,
    line_end: bool,
}

struct ViewportRow<'a> {
    line: LineView<'a>,
    char_count: usize,
    completed: Option<Cow<'a, str>>,
}

pub(super) struct ViewportRead<'a> {
    rows: Vec<ViewportRow<'a>>,
    start_row: usize,
    start_col: usize,
    fetch_width: usize,
    content_height: usize,
    content_width: usize,
    line_gutter: usize,
    external_gutter: usize,
}

pub(super) struct RowPlan {
    rows: Vec<PlannedRow>,
    pub(super) content_height: usize,
    content_width: usize,
    line_gutter: usize,
    external_gutter: usize,
}

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
    let visible_lines = buffer.try_visible_lines_window_with_char_counts(
        start_row,
        content_height,
        start_col,
        fetch_width,
    )?;
    let cursor_cells = write_rows(
        out,
        buffer,
        &visible_lines,
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
        cursor,
        cursor_cells,
        viewport,
        gutter,
        content_height,
        visible_lines
            .get(cursor.row.saturating_sub(start_row))
            .map(|(_, char_count)| *char_count),
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

pub(super) fn read_viewport<'a>(
    buffer: &'a dyn Buffer,
    viewport: RenderViewport,
    options: RenderOptions<'_>,
) -> io::Result<ViewportRead<'a>> {
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
    let fetch_width = content_width.saturating_mul(4).saturating_add(32);
    let rows = buffer
        .try_visible_lines_window_with_char_counts(
            start_row,
            content_height,
            start_col,
            fetch_width,
        )?
        .into_iter()
        .map(|(line, char_count)| ViewportRow {
            line,
            char_count,
            completed: None,
        })
        .collect();
    Ok(ViewportRead {
        rows,
        start_row,
        start_col,
        fetch_width,
        content_height,
        content_width,
        line_gutter,
        external_gutter,
    })
}

pub(super) fn plan_buffer_from_read<'a>(
    buffer: &'a dyn Buffer,
    read: &mut ViewportRead<'a>,
) -> io::Result<RowPlan> {
    let line_count = buffer.line_count();
    let mut rows = Vec::with_capacity(read.content_height);
    for screen_row in 0..read.content_height {
        let document_row = read.start_row.saturating_add(screen_row);
        let line_len = read.rows.get(screen_row).map_or(0, |row| row.char_count);
        let mut layout = VisibleLineLayout::default();
        let content_fingerprint = if read.content_width > 0 && document_row < line_count {
            let Some(row) = read.rows.get_mut(screen_row) else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "viewport read omitted an indexed buffer row",
                ));
            };
            row.completed = super::boundary_complete_line_from_initial(
                buffer,
                document_row,
                read.start_col,
                read.fetch_width,
                read.content_width,
                false,
                line_len,
                &row.line.content,
                &mut layout,
            )?;
            let content = row.completed.as_deref().unwrap_or(&row.line.content);
            Some(super::presentation::content_fingerprint(
                &content[..layout.source_byte_len()],
            ))
        } else {
            None
        };
        let end_col = read.start_col.saturating_add(layout.source_scalar_len());
        rows.push(PlannedRow {
            document_row,
            start_col: read.start_col,
            content_fingerprint,
            layout,
            line_end: document_row < line_count && end_col >= line_len,
        });
    }
    Ok(RowPlan {
        rows,
        content_height: read.content_height,
        content_width: read.content_width,
        line_gutter: read.line_gutter,
        external_gutter: read.external_gutter,
    })
}

pub(super) fn complete_read_for_plan<'a>(
    buffer: &'a dyn Buffer,
    plan: &RowPlan,
    read: &mut ViewportRead<'a>,
) -> io::Result<()> {
    for (row_index, planned) in plan.rows.iter().enumerate() {
        if planned.content_fingerprint.is_none() {
            continue;
        }
        let Some(row) = read.rows.get_mut(row_index) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "viewport read omitted a retained buffer row",
            ));
        };
        if row.line.content.len() >= planned.layout.source_byte_len() {
            continue;
        }
        let mut layout = VisibleLineLayout::default();
        row.completed = super::boundary_complete_line_from_initial(
            buffer,
            planned.document_row,
            read.start_col,
            read.fetch_width,
            read.content_width,
            false,
            row.char_count,
            &row.line.content,
            &mut layout,
        )?;
    }
    Ok(())
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

pub(super) fn compose_row(
    out: &mut Vec<u8>,
    plan: &RowPlan,
    read: &ViewportRead<'_>,
    row_index: usize,
    options: RenderOptions<'_>,
    boundaries: &mut Vec<usize>,
) -> io::Result<()> {
    let Some(row) = plan.rows.get(row_index) else {
        return Ok(());
    };
    style::write_row_start(
        out,
        row_index.saturating_add(1),
        options.theme.text,
        options.theme.truecolor,
    )?;
    if plan.external_gutter > 0 {
        write_external_change_gutter(
            out,
            row.document_row,
            options.external_changes,
            options.theme,
        )?;
    }
    if plan.line_gutter > 0 {
        write_line_number(out, row.document_row, plan.line_gutter, options.theme)?;
    }
    if row.content_fingerprint.is_some() {
        let content = planned_content(read, row_index, row)?;
        style::write_content_line_from_layout(
            out,
            content,
            row.document_row,
            row.start_col,
            options,
            &row.layout,
            boundaries,
        )?;
    }
    style::write_reset(out)
}

pub(super) fn row_fingerprint(plan: &RowPlan, row_index: usize, options: RenderOptions<'_>) -> u64 {
    let Some(row) = plan.rows.get(row_index) else {
        return super::presentation::empty_row_fingerprint(options);
    };
    super::presentation::row_fingerprint(
        options,
        row.document_row,
        row.start_col,
        row.content_fingerprint,
        row.layout.byte_len(),
        row.layout.scalar_len(),
        false,
        row.line_end,
    )
}

impl RowPlan {
    pub(super) fn cursor_position(&self, cursor: Cursor) -> Option<(usize, usize)> {
        let (row_index, row) = self
            .rows
            .iter()
            .enumerate()
            .find(|(_, row)| row.document_row == cursor.row)?;
        row.content_fingerprint?;
        if cursor.col < row.start_col || self.content_width == 0 {
            return None;
        }
        let end_col = row.start_col.saturating_add(row.layout.source_scalar_len());
        if cursor.col >= end_col && !(row.line_end && cursor.col == end_col) {
            return None;
        }
        let cells = row
            .layout
            .scalar_to_cell(cursor.col.saturating_sub(row.start_col));
        if cells > self.content_width || (cells == self.content_width && !row.line_end) {
            return None;
        }
        let gutter = self.line_gutter.saturating_add(self.external_gutter);
        Some((
            row_index.saturating_add(1),
            gutter
                .saturating_add(cells)
                .saturating_add(1)
                .min(gutter.saturating_add(self.content_width).max(1)),
        ))
    }
}

fn planned_content<'a>(
    read: &'a ViewportRead<'_>,
    row_index: usize,
    row: &PlannedRow,
) -> io::Result<&'a str> {
    let content = read
        .rows
        .get(row_index)
        .map(|row| row.completed.as_deref().unwrap_or(&row.line.content))
        .unwrap_or_default();
    let byte_len = row.layout.source_byte_len();
    if content.len() < byte_len || !content.is_char_boundary(byte_len) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "buffer changed while composing retained row",
        ));
    }
    let content = &content[..byte_len];
    if row.content_fingerprint != Some(super::presentation::content_fingerprint(content)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "buffer changed while composing retained row",
        ));
    }
    Ok(content)
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
    visible_lines: &[(crate::buffer::LineView<'_>, usize)],
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
    let mut layout = VisibleLineLayout::default();
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
            if let Some((line, line_len)) = visible_lines.get(screen_row - 1) {
                let completed = super::boundary_complete_line_from_initial(
                    buffer,
                    document_row,
                    start_col,
                    fetch_width,
                    width,
                    false,
                    *line_len,
                    &line.content,
                    &mut layout,
                )?;
                let content = completed.as_deref().unwrap_or(&line.content);
                style::write_content_line_from_layout(
                    out,
                    content,
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
    cursor: Cursor,
    cursor_cells: usize,
    viewport: RenderViewport,
    gutter: usize,
    content_height: usize,
    line_end: Option<usize>,
) -> Option<(usize, usize)> {
    let content_width = viewport.width.saturating_sub(gutter);
    let Cursor { row, col } = cursor;
    let row_visible =
        row >= viewport.start_row && row < viewport.start_row.saturating_add(content_height);
    let line_end = line_end.unwrap_or(0);
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

#[cfg(test)]
mod retained_tests {
    use super::*;
    use crate::buffer::{CompactLineStarts, PreviewBuffer};

    #[test]
    fn planned_content_preserves_a_borrowed_preview_slice() {
        let buffer =
            PreviewBuffer::from_parts("borrowed preview row".into(), CompactLineStarts::new());
        let mut read = read_viewport(
            &buffer,
            RenderViewport::new(0, 0, 2, 8),
            RenderOptions::default(),
        )
        .unwrap();
        let plan = plan_buffer_from_read(&buffer, &mut read).unwrap();
        assert!(matches!(
            read.rows[0].line.content,
            Cow::Borrowed("borrowed preview row")
        ));
        let content = planned_content(&read, 0, &plan.rows[0]).unwrap();
        assert_eq!(content, "borrowed");
    }
}
