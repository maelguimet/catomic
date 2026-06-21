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
/// Clears, writes visible content, positions cursor at buffer cursor.
/// Uses char-index columns (per Phase 0 col semantics decision).
pub fn render_buffer<W: Write + ?Sized>(
    out: &mut W,
    buffer: &dyn Buffer,
    _start_row: usize,
    _height: usize,
) -> std::io::Result<()> {
    // Full clear + home for Phase 0 simplicity.
    write!(out, "\x1b[2J\x1b[1;1H")?;

    for line in buffer.lines() {
        writeln!(out, "{}", line)?;
    }

    let Cursor { row, col } = buffer.cursor();
    // 1-based ANSI coordinates. col is char index (scalar), not wcwidth.
    let r = row.saturating_add(1);
    let c = col.saturating_add(1);
    write!(out, "\x1b[{};{}H", r, c)?;
    out.flush()?;
    Ok(())
}

// TODO: syntax highlight stubs, markdown rendering (pulldown-cmark + custom ANSI).
