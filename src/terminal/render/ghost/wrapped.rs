//! Purpose: compose soft-wrapped rows for a virtual inline ghost insertion.
//! Owns: bounded source-row wrapping, ghost-range slicing, and wrapped cursor placement.
//! Must not: mutate buffers, render unwrapped viewports, or materialize full documents.
//! Invariants: splits stay on grapheme boundaries; ghost-created lines have blank gutters.
//! Phase: post-v0.1 opt-in inline autocomplete.

use super::*;

#[derive(Debug)]
struct DisplayRow {
    content: String,
    ghost: Option<(usize, usize)>,
    document_row: usize,
    start_col: usize,
    line_number: bool,
    cursor_col: Option<usize>,
}

pub(super) fn compose_buffer(
    out: &mut Vec<u8>,
    buffer: &dyn Buffer,
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions<'_>,
    ghost: GhostText<'_>,
) -> io::Result<()> {
    let height = viewport.height.saturating_sub(1);
    let (line_gutter, change_gutter) = gutters(buffer, viewport.width, options);
    let gutter = line_gutter.saturating_add(change_gutter);
    let width = viewport.width.saturating_sub(gutter);
    let mut rows = Vec::with_capacity(height);
    let mut document_row = viewport.start_row;
    while rows.len() < height && document_row < buffer.line_count() {
        let start_col = if document_row == viewport.start_row {
            viewport.wrap_col
        } else {
            0
        };
        if document_row == ghost.cursor.row && start_col <= ghost.cursor.col {
            append_wrapped_overlay(
                buffer,
                ghost,
                document_row,
                start_col,
                height - rows.len(),
                width,
                &mut rows,
            )?;
        } else {
            append_normal_wrapped(
                buffer,
                document_row,
                start_col,
                height - rows.len(),
                width,
                &mut rows,
            )?;
        }
        document_row += 1;
    }
    write_display_rows(
        out,
        &rows,
        height,
        width,
        line_gutter,
        change_gutter,
        options,
    )?;
    write_status(out, viewport, message, options)?;
    let (row, col) = rows
        .iter()
        .enumerate()
        .find_map(|(index, row)| {
            row.cursor_col.map(|cursor| {
                let cell = text_layout::scalar_to_cell(&row.content, cursor);
                (index + 1, gutter.saturating_add(cell).saturating_add(1))
            })
        })
        .unwrap_or((1, gutter.saturating_add(1)));
    super::super::write_terminal_cursor(
        out,
        Some((row, col.min(viewport.width.max(1)))),
        options.cursor_shape,
    )
}

fn append_normal_wrapped(
    buffer: &dyn Buffer,
    row: usize,
    start_col: usize,
    limit: usize,
    width: usize,
    output: &mut Vec<DisplayRow>,
) -> io::Result<()> {
    let mut wrapped = Vec::new();
    super::super::wrapped::append_line_rows(buffer, row, start_col, limit, width, &mut wrapped)?;
    output.extend(wrapped.into_iter().map(|line| DisplayRow {
        line_number: line.start_col == 0,
        document_row: line.document_row,
        start_col: line.start_col,
        content: line.content,
        ghost: None,
        cursor_col: None,
    }));
    Ok(())
}

fn append_wrapped_overlay(
    buffer: &dyn Buffer,
    ghost: GhostText<'_>,
    row: usize,
    start_col: usize,
    limit: usize,
    width: usize,
    output: &mut Vec<DisplayRow>,
) -> io::Result<()> {
    let local_cursor = ghost.cursor.col.saturating_sub(start_col);
    let capacity = width.saturating_mul(limit).saturating_mul(4);
    let fetch = local_cursor.saturating_add(capacity).saturating_add(32);
    let source = buffer
        .try_visible_lines_window(row, 1, start_col, fetch)?
        .into_iter()
        .next()
        .map(|line| line.content)
        .unwrap_or_default();
    let overlay = overlay_lines(&source, local_cursor, ghost.text, 0);
    let target_len = output.len().saturating_add(limit);
    for (index, line) in overlay.into_iter().enumerate() {
        if output.len() >= target_len {
            break;
        }
        append_string_wraps(
            line,
            row,
            if index == 0 { start_col } else { 0 },
            target_len - output.len(),
            width,
            (index == 0).then_some(local_cursor),
            output,
        );
    }
    Ok(())
}

fn append_string_wraps(
    line: OverlayLine,
    document_row: usize,
    base_col: usize,
    limit: usize,
    width: usize,
    cursor: Option<usize>,
    output: &mut Vec<DisplayRow>,
) {
    let total = line.content.chars().count();
    let mut start = 0usize;
    for index in 0..limit {
        if start >= total || width == 0 {
            output.push(display_slice(
                &line,
                document_row,
                base_col,
                start,
                start,
                index,
                cursor,
            ));
            break;
        }
        let remaining: String = line.content.chars().skip(start).collect();
        let mut take = text_layout::clipped_scalar_len(&remaining, width);
        if take == 0 {
            take = text_layout::next_grapheme_col(&remaining, 0);
        }
        let end = start.saturating_add(take).min(total);
        output.push(display_slice(
            &line,
            document_row,
            base_col,
            start,
            end,
            index,
            cursor,
        ));
        start = end;
        if start >= total {
            break;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn display_slice(
    line: &OverlayLine,
    document_row: usize,
    base_col: usize,
    start: usize,
    end: usize,
    index: usize,
    cursor: Option<usize>,
) -> DisplayRow {
    DisplayRow {
        content: line.content.chars().skip(start).take(end - start).collect(),
        ghost: intersect(line.ghost, start, end),
        document_row,
        start_col: base_col.saturating_add(start),
        line_number: line.line_number && index == 0,
        cursor_col: cursor
            .filter(|cursor| {
                *cursor >= start && (*cursor < end || end == line.content.chars().count())
            })
            .map(|cursor| cursor.saturating_sub(start)),
    }
}

fn write_display_rows(
    out: &mut Vec<u8>,
    rows: &[DisplayRow],
    height: usize,
    width: usize,
    line_gutter: usize,
    change_gutter: usize,
    options: RenderOptions<'_>,
) -> io::Result<()> {
    for screen_row in 1..=height {
        super::super::style::write_row_start(
            out,
            screen_row,
            options.theme.text,
            options.theme.truecolor,
        )?;
        let Some(row) = rows.get(screen_row - 1) else {
            continue;
        };
        write_row_prefix(
            out,
            row.document_row,
            line_gutter,
            change_gutter,
            row.line_number,
            options,
        )?;
        super::super::style::write_content_line_with_ghost(
            out,
            &row.content,
            row.document_row,
            row.start_col,
            width.max(1),
            options,
            row.ghost,
        )?;
    }
    Ok(())
}
