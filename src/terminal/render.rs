//! Purpose: render the visible buffer viewport with direct ANSI writes.
//! Owns: row clearing, visible styled text, bottom annotation, and cursor placement.
//! Must not: mutate editor/buffer state, read full buffers, or own terminal setup.
//! Invariants: file-backed read errors propagate before output; every viewport row is cleared;
//!   rendering never emits a full-screen clear.
//! Phase: 4-a viewport-only syntax styling.

use std::io::Write;

use crate::buffer::{Buffer, Cursor};
use crate::editor::syntax::SyntaxKind;

mod style;
pub(crate) mod wrapped;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextHighlight {
    pub(crate) start: Cursor,
    pub(crate) end: Cursor,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct RenderOptions {
    pub(crate) highlight: Option<TextHighlight>,
    pub(crate) syntax: SyntaxKind,
    pub(crate) line_numbers: bool,
    pub(crate) whitespace: bool,
    pub(crate) soft_wrap: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RenderViewport {
    start_row: usize,
    start_col: usize,
    height: usize,
    width: usize,
    wrap_col: usize,
}

impl RenderViewport {
    pub const fn new(start_row: usize, start_col: usize, height: usize, width: usize) -> Self {
        Self {
            start_row,
            start_col,
            height,
            width,
            wrap_col: 0,
        }
    }

    pub(crate) const fn with_wrap_col(mut self, wrap_col: usize) -> Self {
        self.wrap_col = wrap_col;
        self
    }
}

pub(crate) fn line_number_gutter(line_count: usize) -> usize {
    line_count.max(1).to_string().len().saturating_add(1)
}

/// Basic viewport render with one optional active search highlight.
/// Clears each viewport row, writes the visible window using visible_lines
/// (not the full .lines() clone), positions the terminal cursor exactly at
/// the buffer's logical cursor. No phantom line is appended after the last
/// rendered row.
///
/// `viewport` defines the visible row/column origin and terminal dimensions.
/// Bottom row (height) reserved for minimal message if provided; content uses height-1.
/// Horizontal slicing starts at a scalar document column but clips by terminal cells.
/// Least invasive addition: message shown on last row via absolute positioning.
pub fn render_buffer<W: Write + ?Sized>(
    out: &mut W,
    buffer: &dyn Buffer,
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions,
) -> std::io::Result<()> {
    if options.soft_wrap {
        return wrapped::render_buffer(out, buffer, viewport, message, options);
    }
    let RenderViewport {
        start_row,
        start_col,
        height,
        width,
        ..
    } = viewport;
    // Reserve bottom row for message/status (matches screen.visible_height intent).
    // Horizontal: use width directly as content width (no sidebar/status reservation).
    let content_h = height.saturating_sub(1);
    let gutter = if options.line_numbers {
        line_number_gutter(buffer.line_count())
    } else {
        0
    }
    .min(width);
    let content_w = width.saturating_sub(gutter);
    let cursor = buffer.cursor();
    let cursor_window =
        if cursor.row >= start_row && cursor.row < start_row.saturating_add(content_h) {
            cursor.col.saturating_sub(start_col).saturating_add(1)
        } else {
            0
        };
    let fetch_width = content_w
        .saturating_mul(4)
        .saturating_add(32)
        .max(cursor_window);
    let visible = buffer.try_visible_lines_window(start_row, content_h, start_col, fetch_width)?;
    for screen_row in 1..=content_h {
        write!(out, "\x1b[{};1H\x1b[K", screen_row)?;
        if gutter > 0 {
            write_line_number(out, start_row + screen_row - 1, gutter)?;
        }
        if content_w > 0 {
            if let Some(line) = visible.get(screen_row - 1) {
                style::write_content_line(
                    out,
                    &line.content,
                    start_row + screen_row - 1,
                    start_col,
                    content_w,
                    options,
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
    let Cursor { row, col } = cursor;
    let screen_row = if row >= start_row {
        row - start_row + 1
    } else {
        1
    };
    let cursor_cells = if row >= start_row && row < start_row.saturating_add(content_h) {
        visible
            .get(row - start_row)
            .map(|line| {
                crate::editor::text_layout::scalar_to_cell(
                    &line.content,
                    col.saturating_sub(start_col),
                )
            })
            .unwrap_or(0)
    } else {
        0
    };
    let screen_col = gutter
        .saturating_add(cursor_cells)
        .saturating_add(1)
        .min(width.max(1));
    write!(out, "\x1b[{};{}H", screen_row, screen_col)?;
    out.flush()?;
    Ok(())
}

pub(super) fn write_line_number<W: Write + ?Sized>(
    out: &mut W,
    row: usize,
    gutter: usize,
) -> std::io::Result<()> {
    let label = format!(
        "{:>width$} ",
        row.saturating_add(1),
        width = gutter.saturating_sub(1)
    );
    let clipped: String = label.chars().take(gutter).collect();
    write!(out, "\x1b[2;90m{clipped}\x1b[0m")
}

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
            RenderViewport::new(0, 0, 3, 20),
            None,
            RenderOptions {
                highlight: Some(TextHighlight {
                    start: Cursor { row: 0, col: 5 },
                    end: Cursor { row: 0, col: 11 },
                }),
                ..RenderOptions::default()
            },
        )
        .unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("zero \x1b[7mtarget\x1b[27m here"));
    }

    #[test]
    fn render_buffer_highlights_a_multiline_selection() {
        let b = SimpleBuffer::from_text("zero here\nmiddle\nlast row");
        let mut out = Vec::new();

        render_buffer(
            &mut out,
            &b,
            RenderViewport::new(0, 0, 4, 20),
            None,
            RenderOptions {
                highlight: Some(TextHighlight {
                    start: Cursor { row: 0, col: 5 },
                    end: Cursor { row: 2, col: 4 },
                }),
                ..RenderOptions::default()
            },
        )
        .unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("zero \x1b[7mhere\x1b[27m"));
        assert!(rendered.contains("\x1b[7mmiddle\x1b[27m"));
        assert!(rendered.contains("\x1b[7mlast\x1b[27m row"));
    }

    #[test]
    fn render_buffer_height_zero_no_bottom_pos_and_no_panic() {
        let b = SimpleBuffer::from_text("hello\nworld\n");
        let mut out: Vec<u8> = Vec::new();
        // Must not panic
        render_buffer(
            &mut out,
            &b,
            RenderViewport::new(0, 0, 0, 10),
            None,
            RenderOptions::default(),
        )
        .expect("render h=0");
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
        render_buffer(
            &mut out,
            &b,
            RenderViewport::new(0, 0, 1, 10),
            Some("msg"),
            RenderOptions::default(),
        )
        .expect("render h=1");
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
        render_buffer(
            &mut out,
            &b,
            RenderViewport::new(0, 0, 3, 0),
            None,
            RenderOptions::default(),
        )
        .expect("render w=0");
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

        render_buffer(
            &mut out,
            &b,
            RenderViewport::new(0, 0, 4, 10),
            Some("status"),
            RenderOptions::default(),
        )
        .unwrap();

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
        render_buffer(
            &mut out_default,
            &b,
            RenderViewport::new(0, 0, 4, 6),
            None,
            RenderOptions::default(),
        )
        .expect("default");
        let default_s = String::from_utf8_lossy(&out_default);

        let mut out_explicit: Vec<u8> = Vec::new();
        render_buffer(
            &mut out_explicit,
            &b,
            RenderViewport::new(0, 0, 4, 6),
            None,
            RenderOptions::default(),
        )
        .expect("explicit");
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
    fn render_buffer_horizontal_cell_clipping_preserves_grapheme_boundaries() {
        // The three-cell viewport fits é (one cell) and 猫 (two cells).
        let b = SimpleBuffer::from_text("aé猫🙂Z\n");
        let mut out: Vec<u8> = Vec::new();
        render_buffer(
            &mut out,
            &b,
            RenderViewport::new(0, 1, 2, 3),
            None,
            RenderOptions::default(),
        )
        .expect("render slice multibyte");
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("é猫"),
            "expected cell-clipped Unicode content: {}",
            s
        );
        assert!(!s.contains("a"), "should have skipped the first scalar");
        assert!(
            !s.contains('🙂'),
            "wide emoji does not fit in remaining cells"
        );
        assert!(!s.contains('Z'), "content past the cell limit stays hidden");
    }

    #[test]
    fn render_buffer_cursor_uses_grapheme_display_width() {
        let mut b = SimpleBuffer::from_text("a\u{301}猫x");
        b.set_cursor(Cursor { row: 0, col: 3 });
        let mut out = Vec::new();

        render_buffer(
            &mut out,
            &b,
            RenderViewport::new(0, 0, 2, 8),
            None,
            RenderOptions::default(),
        )
        .unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("a\u{301}猫x"));
        assert!(rendered.ends_with("\x1b[1;4H"));
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
        let err = render_buffer(
            &mut out,
            &buffer,
            RenderViewport::new(0, 0, 2, 8),
            None,
            RenderOptions::default(),
        )
        .expect_err("render must surface changed backing file");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

        let _ = std::fs::remove_file(path);
    }
}
