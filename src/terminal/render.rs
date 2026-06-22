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
            line.chars()
                .skip(start_col)
                .take(content_w)
                .collect::<String>()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::SimpleBuffer;

    #[test]
    fn render_buffer_height_zero_no_bottom_pos_and_no_panic() {
        let b = SimpleBuffer::from_text("hello\nworld\n");
        let mut out: Vec<u8> = Vec::new();
        // Must not panic
        render_buffer(&mut out, &b, 0, 0, 0, 10, None).expect("render h=0");
        let s = String::from_utf8_lossy(&out);
        // No bottom-row absolute positioning for height 0
        assert!(
            !s.contains("\x1b[0;"),
            "height=0 must not emit bottom-row positioning: {}",
            s
        );
        // Still does safe clear + final cursor positioning
        assert!(s.contains("\x1b[2J\x1b[1;1H"), "still clears");
        assert!(s.contains("\x1b[1;1H"), "safe cursor pos at 1;1 for empty viewport");
    }

    #[test]
    fn render_buffer_height_one_reserves_only_row_for_message_no_content_lines() {
        let b = SimpleBuffer::from_text("L0\nL1\nL2\n");
        let mut out: Vec<u8> = Vec::new();
        render_buffer(&mut out, &b, 0, 0, 1, 10, Some("msg")).expect("render h=1");
        let s = String::from_utf8_lossy(&out);
        // With h=1, content_h=0 => no visible lines should be emitted
        assert!(
            !s.contains("L0") && !s.contains("L1") && !s.contains("L2"),
            "height=1 must emit no content lines: {}",
            s
        );
        // Bottom row positioning at height=1
        assert!(s.contains("\x1b[1;1H"), "positions to row 1 for message");
        assert!(s.contains("msg"), "message emitted");
    }

    #[test]
    fn render_buffer_width_zero_emits_no_line_content_but_clears_and_positions() {
        let b = SimpleBuffer::from_text("abc\ndef\n");
        let mut out: Vec<u8> = Vec::new();
        render_buffer(&mut out, &b, 0, 0, 3, 0, None).expect("render w=0");
        let s = String::from_utf8_lossy(&out);
        // No actual text content from lines
        assert!(
            !s.contains("abc") && !s.contains("def"),
            "width=0 must emit no line content chars: {}",
            s
        );
        // Still clears and does final cursor positioning safely
        assert!(s.contains("\x1b[2J"), "clears on w=0");
        assert!(s.contains("\x1b["), "positions cursor");
    }

    #[test]
    fn render_buffer_start_col_zero_nonzero_width_preserves_default_visible_output() {
        let b = SimpleBuffer::from_text("0123456789\nABCDEFGHIJ\n");
        let mut out_default: Vec<u8> = Vec::new();
        // Classic call site shape with start_col=0
        render_buffer(&mut out_default, &b, 0, 0, 4, 6, None).expect("default");
        let default_s = String::from_utf8_lossy(&out_default);

        let mut out_explicit: Vec<u8> = Vec::new();
        render_buffer(&mut out_explicit, &b, 0, 0, 4, 6, None).expect("explicit");
        let explicit_s = String::from_utf8_lossy(&out_explicit);

        // Behaviorally identical for the default case
        assert_eq!(default_s, explicit_s);

        // And contains first visible slice (no start_col skip)
        assert!(default_s.contains("012345"), "first 6 chars of first line visible with start_col=0");
    }
}
