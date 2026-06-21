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

/// Very basic render used while we bootstrap.
/// Real implementation will be more sophisticated.
pub fn render_buffer<W: Write>(
    out: &mut W,
    buffer: &dyn Buffer,
    _start_row: usize,
    _height: usize,
) -> std::io::Result<()> {
    // Clear is done by caller in Phase 0 app.
    for line in buffer.lines() {
        writeln!(out, "{}", line)?;
    }

    let Cursor { row, col } = buffer.cursor();
    // Move cursor to the right place (very naive).
    // Real code will use crossterm cursor movement and account for line wrapping etc.
    write!(out, "\x1b[{};{}H", row + 1, col + 1)?;
    out.flush()?;
    Ok(())
}

// TODO: syntax highlight stubs, markdown rendering (pulldown-cmark + custom ANSI).
