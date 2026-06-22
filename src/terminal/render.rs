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
/// start/start_col/height/width define the viewport slice.
/// Bottom row (height) reserved for minimal message if provided; content uses height-1.
/// For horizontal: scalar char slicing from start_col, at most width chars.
/// Least invasive addition: message shown on last row via absolute positioning.
pub fn render_buffer<W: Write + ?Sized>(
    out: &mut W,
    buffer: &dyn Buffer,
    start: usize,
    start_col: usize,
    height: usize,
    width: usize,
    message: Option<&str>,
) -> std::io::Result<()> {
    // Full clear + home for Phase 0 simplicity (no partial redraw yet).
    write!(out, "\x1b[2J\x1b[1;1H")?;

    // Reserve bottom row for message/status (matches screen.visible_height intent).
    // Horizontal: use width directly as content width (no sidebar/status reservation).
    let content_h = height.saturating_sub(1);
    let content_w = width;
    let visible = buffer.visible_lines(start, content_h);
    for (i, lv) in visible.iter().enumerate() {
        if i > 0 {
            write!(out, "\r\n")?;
        }
        let line = &lv.content;
        let rendered = if content_w == 0 {
            String::new()
        } else {
            line.chars().skip(start_col).take(content_w).collect::<String>()
        };
        write!(out, "{}", rendered)?;
    }

    // Minimal bottom message line on last row (pinned via absolute move).
    // Shows message text if present (error, quit warning, etc.).
    // When no message, still emit to clear prior content from bottom row.
    if height > 0 {
        let msg = message.unwrap_or("");
        write!(out, "\x1b[{};1H\x1b[K{}", height, msg)?;
    }

    // Position cursor relative to the rendered viewport (content area).
    // Horizontal scroll: screen col = (buffer col - start_col) + 1 (1-based).
    // Saturating math so it never panics/underflows.
    // If width is 0 still emit safe cursor position.
    let Cursor { row, col } = buffer.cursor();
    let screen_row = if row >= start { row - start + 1 } else { 1 };
    let screen_col = col.saturating_sub(start_col).saturating_add(1);
    write!(out, "\x1b[{};{}H", screen_row, screen_col)?;
    out.flush()?;
    Ok(())
}

// TODO: syntax highlight stubs, markdown rendering (pulldown-cmark + custom ANSI).
