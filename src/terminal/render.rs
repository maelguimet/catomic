//! Dumb ANSI rendering.
//!
//! Phase 0–2 philosophy: direct writes + cursor control.
//! Only later introduce widget libraries if they don't hurt latency.
//!
//! Responsibilities:
//! - Render visible buffer region
//! - Position cursor
//! - Minimal status (filename, mode, dirty?)
//! - Respect large-file limits (no full highlight for huge files)

use std::io::Write;

use crate::buffer::{Buffer, Cursor};

/// Very basic full-screen render for Phase 0.
/// Clears, writes the visible window from the buffer using visible_lines
/// (not the full .lines() clone), positions the terminal cursor exactly at
/// the buffer's logical cursor. No phantom line is appended after the last
/// rendered row.
///
/// start/height define the viewport slice. For Phase 0 start is usually 0.
pub fn render_buffer<W: Write + ?Sized>(
    out: &mut W,
    buffer: &dyn Buffer,
    start: usize,
    height: usize,
) -> std::io::Result<()> {
    // Full clear + home for Phase 0 simplicity (no partial redraw yet).
    write!(out, "\x1b[2J\x1b[1;1H")?;

    let visible = buffer.visible_lines(start, height);
    for (i, lv) in visible.iter().enumerate() {
        if i > 0 {
            write!(out, "\r\n")?;
        }
        write!(out, "{}", lv.content)?;
    }

    // Position cursor relative to the rendered viewport.
    // If cursor is outside the current slice we still emit a position
    // (terminal may clip; Phase 0 has no scroll).
    let Cursor { row, col } = buffer.cursor();
    let screen_row = if row >= start {
        row - start + 1
    } else {
        1
    };
    // 1-based. col is scalar index per Phase 0 decision.
    let screen_col = col.saturating_add(1);
    write!(out, "\x1b[{};{}H", screen_row, screen_col)?;
    out.flush()?;
    Ok(())
}

// TODO: syntax highlight stubs, markdown rendering (pulldown-cmark + custom ANSI).
