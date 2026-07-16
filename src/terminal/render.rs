//! Purpose: render the visible buffer viewport with direct ANSI writes.
//! Owns: row clearing, visible text, bottom status/message output, and cursor placement.
//! Must not: mutate editor/buffer state, read full buffers, or own terminal setup.
//! Invariants: file-backed read errors propagate before output; every viewport row is cleared;
//!   rendering never emits a full-screen clear.
//! Phase: 3-a search highlighting over Phase 2 row-oriented redraw hardening.

use std::io::Write;

use crate::buffer::{Buffer, Cursor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextHighlight {
    pub(crate) row: usize,
    pub(crate) start_col: usize,
    pub(crate) end_col: usize,
}

/// Basic viewport render with one optional active search highlight.
/// Clears each viewport row, writes the visible window using visible_lines
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
    highlight: Option<TextHighlight>,
) -> std::io::Result<()> {
    // Reserve bottom row for message/status (matches screen.visible_height intent).
    // Horizontal: use width directly as content width (no sidebar/status reservation).
    let content_h = height.saturating_sub(1);
    let content_w = width;
    let visible = buffer.try_visible_lines_window(start, content_h, start_col, content_w)?;
    for screen_row in 1..=content_h {
        write!(out, "\x1b[{};1H\x1b[K", screen_row)?;
        if content_w > 0 {
            if let Some(line) = visible.get(screen_row - 1) {
                write_content_line(
                    out,
                    &line.content,
                    start + screen_row - 1,
                    start_col,
                    highlight,
                )?;
            }
        }
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

fn write_content_line<W: Write + ?Sized>(
    out: &mut W,
    content: &str,
    row: usize,
    start_col: usize,
    highlight: Option<TextHighlight>,
) -> std::io::Result<()> {
    let Some(highlight) = highlight.filter(|highlight| highlight.row == row) else {
        return write!(out, "{content}");
    };
    let content_len = content.chars().count();
    let visible_end = start_col.saturating_add(content_len);
    let overlap_start = highlight.start_col.max(start_col);
    let overlap_end = highlight.end_col.min(visible_end);
    if overlap_start >= overlap_end {
        return write!(out, "{content}");
    }
    let local_start = overlap_start - start_col;
    let local_end = overlap_end - start_col;
    let prefix: String = content.chars().take(local_start).collect();
    let matched: String = content
        .chars()
        .skip(local_start)
        .take(local_end - local_start)
        .collect();
    let suffix: String = content.chars().skip(local_end).collect();
    write!(out, "{prefix}\x1b[7m{matched}\x1b[27m{suffix}")
}

// TODO: syntax highlight stubs, markdown rendering (pulldown-cmark + custom ANSI).

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::{LargeFileBuffer, SimpleBuffer};

    #[test]
    fn render_buffer_highlights_the_visible_search_match() {
        let b = SimpleBuffer::from_text("zero target here\n");
        let mut out = Vec::new();

        render_buffer(
            &mut out,
            &b,
            0,
            0,
            3,
            20,
            None,
            Some(TextHighlight {
                row: 0,
                start_col: 5,
                end_col: 11,
            }),
        )
        .unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("zero \x1b[7mtarget\x1b[27m here"));
    }

    #[test]
    fn render_buffer_height_zero_no_bottom_pos_and_no_panic() {
        let b = SimpleBuffer::from_text("hello\nworld\n");
        let mut out: Vec<u8> = Vec::new();
        // Must not panic
        render_buffer(&mut out, &b, 0, 0, 0, 10, None, None).expect("render h=0");
        let s = String::from_utf8_lossy(&out);
        // No bottom-row absolute positioning for height 0
        assert!(
            !s.contains("\x1b[0;"),
            "height=0 must not emit bottom-row positioning: {}",
            s
        );
        // No screen-wide clear; final cursor positioning remains safe.
        assert!(!s.contains("\x1b[2J"), "must not clear whole screen");
        assert!(
            s.contains("\x1b[1;1H"),
            "safe cursor pos at 1;1 for empty viewport"
        );
    }

    #[test]
    fn render_buffer_height_one_reserves_only_row_for_message_no_content_lines() {
        let b = SimpleBuffer::from_text("L0\nL1\nL2\n");
        let mut out: Vec<u8> = Vec::new();
        render_buffer(&mut out, &b, 0, 0, 1, 10, Some("msg"), None).expect("render h=1");
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
    fn render_buffer_width_zero_emits_no_content_but_clears_rows_and_positions() {
        let b = SimpleBuffer::from_text("abc\ndef\n");
        let mut out: Vec<u8> = Vec::new();
        render_buffer(&mut out, &b, 0, 0, 3, 0, None, None).expect("render w=0");
        let s = String::from_utf8_lossy(&out);
        // No actual text content from lines
        assert!(
            !s.contains("abc") && !s.contains("def"),
            "width=0 must emit no line content chars: {}",
            s
        );
        // Still clears viewport rows and does final cursor positioning safely.
        assert!(s.contains("\x1b[1;1H\x1b[K"), "clears first content row");
        assert!(s.contains("\x1b[2;1H\x1b[K"), "clears second content row");
        assert!(!s.contains("\x1b[2J"), "does not clear whole screen");
        assert!(s.contains("\x1b["), "positions cursor");
    }

    #[test]
    fn render_buffer_clears_each_row_without_full_screen_clear() {
        let b = SimpleBuffer::from_text("only");
        let mut out = Vec::new();

        render_buffer(&mut out, &b, 0, 0, 4, 10, Some("status"), None).unwrap();

        let s = String::from_utf8(out).unwrap();
        assert!(!s.contains("\x1b[2J"));
        for row in 1..=3 {
            assert!(s.contains(&format!("\x1b[{row};1H\x1b[K")));
        }
        assert!(s.contains("\x1b[4;1H\x1b[Kstatus"));
    }

    #[test]
    fn render_buffer_start_col_zero_nonzero_width_preserves_default_visible_output() {
        let b = SimpleBuffer::from_text("0123456789\nABCDEFGHIJ\n");
        let mut out_default: Vec<u8> = Vec::new();
        // Classic call site shape with start_col=0
        render_buffer(&mut out_default, &b, 0, 0, 4, 6, None, None).expect("default");
        let default_s = String::from_utf8_lossy(&out_default);

        let mut out_explicit: Vec<u8> = Vec::new();
        render_buffer(&mut out_explicit, &b, 0, 0, 4, 6, None, None).expect("explicit");
        let explicit_s = String::from_utf8_lossy(&out_explicit);

        // Behaviorally identical for the default case
        assert_eq!(default_s, explicit_s);

        // And contains first visible slice (no start_col skip)
        assert!(
            default_s.contains("012345"),
            "first 6 chars of first line visible with start_col=0"
        );
    }

    #[test]
    fn render_buffer_horizontal_scalar_slicing_multibyte_preserves_char_boundaries() {
        // Unicode scalar (char) slicing: "é" is 1 scalar, "猫" 1, "🙂" 1.
        // start_col/take must not split multibyte sequences.
        let b = SimpleBuffer::from_text("aé猫🙂Z\n");
        let mut out: Vec<u8> = Vec::new();
        // start_col=1, width=3 => take(3) scalars after skip: "é猫🙂"
        render_buffer(&mut out, &b, 0, 1, 2, 3, None, None).expect("render slice multibyte");
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("é猫🙂"),
            "expected scalar slice of multibyte: {}",
            s
        );
        assert!(!s.contains("a"), "should have skipped the first scalar");
        assert!(!s.contains('Z'), "should have taken only 3 scalars");
    }

    #[test]
    fn render_buffer_propagates_file_backed_window_read_error() {
        let path = std::env::temp_dir().join(format!(
            "catomic_render_changed_large_file_{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, "original stable content").unwrap();
        let buffer = LargeFileBuffer::open(&path).unwrap();

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        file.write_all(b"changed").unwrap();
        file.flush().unwrap();
        drop(file);

        let mut out = Vec::new();
        let err = render_buffer(&mut out, &buffer, 0, 0, 2, 8, None, None)
            .expect_err("render must surface changed backing file");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

        let _ = std::fs::remove_file(path);
    }
}
