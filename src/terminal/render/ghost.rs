//! Purpose: compose non-buffer inline ghost text for wrapped and unwrapped viewports.
//! Owns: virtual cursor-line insertion, dim styling ranges, and unchanged cursor placement.
//! Must not: mutate buffers, expose ghost text through document APIs, or scan full files.
//! Invariants: reads stay viewport-bounded; suffix text shifts visually; ANSI is one frame.
//! Phase: post-v0.1 opt-in inline autocomplete.

use std::io::{self, Write};

use crate::buffer::Buffer;
use crate::editor::text_layout;

use super::{
    change_gutter_width, line_number_gutter, write_change_gutter, write_external_change_gutter,
    write_line_number, GhostText, RenderOptions, RenderViewport,
};

mod wrapped;

#[derive(Debug)]
struct OverlayLine {
    content: String,
    ghost: Option<(usize, usize)>,
    line_number: bool,
    start_col: usize,
}

pub(super) fn compose_buffer(
    out: &mut Vec<u8>,
    buffer: &dyn Buffer,
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions,
    ghost: GhostText<'_>,
) -> io::Result<()> {
    if options.soft_wrap {
        wrapped::compose_buffer(out, buffer, viewport, message, options, ghost)
    } else {
        compose_unwrapped(out, buffer, viewport, message, options, ghost)
    }
}

fn compose_unwrapped(
    out: &mut Vec<u8>,
    buffer: &dyn Buffer,
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions,
    ghost: GhostText<'_>,
) -> io::Result<()> {
    let content_height = super::content_height(viewport.height, options.action_bar);
    if ghost.cursor.row < viewport.start_row
        || ghost.cursor.row >= viewport.start_row.saturating_add(content_height)
        || viewport.start_col > ghost.cursor.col
    {
        return super::frame::compose_buffer(out, buffer, viewport, message, options);
    }
    let (line_gutter, external_gutter, llm_gutter) = gutters(buffer, viewport.width, options);
    let gutter = line_gutter
        .saturating_add(external_gutter)
        .saturating_add(llm_gutter);
    let width = viewport.width.saturating_sub(gutter);
    let cursor_window = ghost.cursor.col.saturating_sub(viewport.start_col) + 1;
    let fetch = width
        .saturating_mul(4)
        .saturating_add(32)
        .max(cursor_window);
    let visible = buffer.try_visible_lines_window(
        viewport.start_row,
        content_height,
        viewport.start_col,
        fetch,
    )?;
    let source = visible
        .get(ghost.cursor.row - viewport.start_row)
        .map(|line| line.content.as_str())
        .unwrap_or_default();
    let local_cursor = ghost.cursor.col.saturating_sub(viewport.start_col);
    let cursor_cell = text_layout::scalar_to_cell(source, local_cursor);
    let line_end = buffer.line_char_count(ghost.cursor.row).unwrap_or(0);
    if width == 0 || cursor_cell > width || (cursor_cell == width && ghost.cursor.col != line_end) {
        return super::frame::compose_buffer(out, buffer, viewport, message, options);
    }
    let overlay = overlay_lines(source, local_cursor, ghost.text, viewport.start_col);
    write_unwrapped_rows(
        out,
        &visible,
        &overlay,
        viewport,
        content_height,
        width,
        line_gutter,
        external_gutter,
        llm_gutter,
        options,
        ghost.cursor.row,
    )?;
    write_status(out, viewport, message, options)?;
    let row = ghost.cursor.row - viewport.start_row + 1;
    let col = gutter.saturating_add(cursor_cell).saturating_add(1);
    super::write_terminal_cursor(
        out,
        Some((row, col.min(viewport.width.max(1)))),
        options.cursor_shape,
    )
}

#[allow(clippy::too_many_arguments)]
fn write_unwrapped_rows(
    out: &mut Vec<u8>,
    visible: &[crate::buffer::LineView],
    overlay: &[OverlayLine],
    viewport: RenderViewport,
    height: usize,
    width: usize,
    line_gutter: usize,
    external_gutter: usize,
    llm_gutter: usize,
    options: RenderOptions<'_>,
    cursor_row: usize,
) -> io::Result<()> {
    let cursor_index = cursor_row - viewport.start_row;
    let mut source_index = 0usize;
    let mut overlay_index = 0usize;
    for screen_row in 1..=height {
        super::style::write_row_start(
            out,
            screen_row,
            options.theme.text,
            options.theme.truecolor,
        )?;
        let (content, ghost, document_row, start_col, numbered) =
            if source_index == cursor_index && overlay_index < overlay.len() {
                let line = &overlay[overlay_index];
                overlay_index += 1;
                if overlay_index == overlay.len() {
                    source_index += 1;
                }
                (
                    line.content.as_str(),
                    line.ghost,
                    cursor_row,
                    line.start_col,
                    line.line_number,
                )
            } else if let Some(line) = visible.get(source_index) {
                let row = viewport.start_row + source_index;
                source_index += 1;
                (line.content.as_str(), None, row, viewport.start_col, true)
            } else {
                continue;
            };
        write_row_prefix(
            out,
            document_row,
            line_gutter,
            external_gutter,
            llm_gutter,
            numbered,
            options,
        )?;
        super::style::write_content_line_with_ghost(
            out,
            content,
            document_row,
            start_col,
            width,
            options,
            ghost,
        )?;
    }
    Ok(())
}

fn overlay_lines(source: &str, cursor: usize, ghost: &str, scroll: usize) -> Vec<OverlayLine> {
    let prefix: String = source.chars().take(cursor).collect();
    let suffix: String = source.chars().skip(cursor).collect();
    let parts: Vec<&str> = ghost.split('\n').collect();
    let mut lines = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        let first = index == 0;
        let last = index + 1 == parts.len();
        let mut content = if first { prefix.clone() } else { String::new() };
        let ghost_start = content.chars().count();
        content.push_str(part);
        let ghost_end = content.chars().count();
        if last {
            content.push_str(&suffix);
        }
        let line = OverlayLine {
            content,
            ghost: (ghost_start < ghost_end).then_some((ghost_start, ghost_end)),
            line_number: first,
            start_col: if first { scroll } else { 0 },
        };
        lines.push(if first || scroll == 0 {
            line
        } else {
            slice_overlay(line, scroll)
        });
    }
    lines
}

fn slice_overlay(line: OverlayLine, skip: usize) -> OverlayLine {
    let total = line.content.chars().count();
    OverlayLine {
        content: line.content.chars().skip(skip).collect(),
        ghost: intersect(line.ghost, skip, total),
        line_number: line.line_number,
        start_col: skip,
    }
}

fn intersect(range: Option<(usize, usize)>, start: usize, end: usize) -> Option<(usize, usize)> {
    let (from, to) = range?;
    let from = from.max(start);
    let to = to.min(end);
    (from < to).then_some((from - start, to - start))
}

pub(super) fn gutters(
    buffer: &dyn Buffer,
    width: usize,
    options: RenderOptions<'_>,
) -> (usize, usize, usize) {
    let line_gutter = if options.line_numbers {
        line_number_gutter(buffer.line_count()).min(width)
    } else {
        0
    };
    let external_gutter = change_gutter_width(
        options
            .external_changes
            .is_some_and(|changes| !changes.markers.is_empty()),
    )
    .min(width.saturating_sub(line_gutter));
    let llm_gutter = change_gutter_width(
        options
            .llm_changes
            .is_some_and(|changes| !changes.gutter_lines.is_empty()),
    )
    .min(
        width
            .saturating_sub(line_gutter)
            .saturating_sub(external_gutter),
    );
    (line_gutter, external_gutter, llm_gutter)
}

pub(super) fn write_row_prefix(
    out: &mut Vec<u8>,
    row: usize,
    line_gutter: usize,
    external_gutter: usize,
    llm_gutter: usize,
    numbered: bool,
    options: RenderOptions<'_>,
) -> io::Result<()> {
    if external_gutter > 0 && numbered {
        write_external_change_gutter(out, row, options.external_changes, options.theme)?;
    } else if external_gutter > 0 {
        write!(out, "{:external_gutter$}", "")?;
    }
    if llm_gutter > 0 && numbered {
        write_change_gutter(out, row, options.llm_changes, options.theme)?;
    } else if llm_gutter > 0 {
        write!(out, "{:llm_gutter$}", "")?;
    }
    if line_gutter > 0 && numbered {
        write_line_number(out, row, line_gutter, options.theme)
    } else if line_gutter > 0 {
        super::style::write_styled_text(
            out,
            &" ".repeat(line_gutter),
            options.theme.text.overlay(options.theme.line_number),
            options.theme.truecolor,
        )
    } else {
        Ok(())
    }
}

pub(super) fn write_status(
    out: &mut Vec<u8>,
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions<'_>,
) -> io::Result<()> {
    super::write_bottom_rows(out, viewport, message, options)
}

#[cfg(test)]
mod tests;
