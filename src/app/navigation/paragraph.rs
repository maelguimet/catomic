//! Purpose: calculate Ctrl+Up/Down paragraph-boundary targets.
//! Owns: blank-line paragraph semantics and visual-column preservation.
//! Must not: mutate buffers, selections, viewport state, history, or terminal output.
//! Invariants: targets are clamped scalar coordinates on grapheme boundaries.
//! Phase: post-v0.1 prose navigation.

use std::io;

use crate::buffer::{Buffer, Cursor};
use crate::editor::text_layout;

#[derive(Clone, Copy)]
pub(super) enum Direction {
    Previous,
    Next,
}

pub(super) fn target(app: &super::super::App, direction: Direction) -> io::Result<Cursor> {
    let buffer = &*app.buffer;
    let current = buffer.cursor();
    let target_row = match direction {
        Direction::Previous => previous_row(buffer, current.row)?,
        Direction::Next => next_row(buffer, current.row)?,
    };
    if target_row == current.row {
        return Ok(current);
    }
    preserve_visual_column(buffer, current, target_row)
}

fn previous_row(buffer: &dyn Buffer, current_row: usize) -> io::Result<usize> {
    if !is_blank(buffer, current_row)? {
        let mut start = current_row;
        while start > 0 && !is_blank(buffer, start - 1)? {
            start -= 1;
        }
        if start < current_row {
            return Ok(start);
        }
    }

    let mut row = current_row;
    while row > 0 {
        row -= 1;
        if !is_blank(buffer, row)? {
            while row > 0 && !is_blank(buffer, row - 1)? {
                row -= 1;
            }
            return Ok(row);
        }
    }
    Ok(0)
}

fn next_row(buffer: &dyn Buffer, current_row: usize) -> io::Result<usize> {
    let last_row = buffer.line_count().saturating_sub(1);
    let mut row = current_row.min(last_row);
    if !is_blank(buffer, row)? {
        while row < last_row && !is_blank(buffer, row)? {
            row += 1;
        }
    }
    while row < last_row && is_blank(buffer, row)? {
        row += 1;
    }
    Ok(row)
}

fn preserve_visual_column(
    buffer: &dyn Buffer,
    current: Cursor,
    target_row: usize,
) -> io::Result<Cursor> {
    let source = line_text(buffer, current.row)?;
    let target = line_text(buffer, target_row)?;
    let visual_col = text_layout::scalar_to_cell(&source, current.col);
    Ok(Cursor {
        row: target_row,
        col: text_layout::scalar_at_cell(&target, visual_col),
    })
}

fn is_blank(buffer: &dyn Buffer, row: usize) -> io::Result<bool> {
    Ok(line_text(buffer, row)?.trim().is_empty())
}

fn line_text(buffer: &dyn Buffer, row: usize) -> io::Result<String> {
    let width = buffer.line_char_count(row).unwrap_or(0);
    Ok(buffer
        .try_visible_lines_window(row, 1, 0, width)?
        .into_iter()
        .next()
        .map(|line| line.content)
        .unwrap_or_default())
}

#[cfg(test)]
mod tests;
