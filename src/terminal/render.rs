//! Purpose: render the visible buffer viewport as complete ANSI frames.
//! Owns: frame composition, row clearing, visible styled text, bottom annotation, and cursor placement.
//! Must not: mutate editor/buffer state, read full buffers, or own terminal setup.
//! Invariants: file-backed read errors propagate before output; every viewport row is cleared;
//!   a frame is committed in one write; rendering never emits a full-screen clear.
//! Phase: 4-a viewport-only syntax styling.

use std::io::{self, Write};

use crate::buffer::{Buffer, Cursor};
use crate::editor::syntax::SyntaxKind;
use crate::terminal::cursor_style::{self, CursorShape};

#[cfg(test)]
mod cursor_tests;
mod status_bar;
mod style;
pub(crate) mod wrapped;

pub(crate) use status_bar::{StatusRole, StatusTheme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TextHighlight {
    pub(crate) start: Cursor,
    pub(crate) end: Cursor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RenderOptions {
    pub(crate) cursor_shape: CursorShape,
    pub(crate) highlight: Option<TextHighlight>,
    pub(crate) syntax: SyntaxKind,
    pub(crate) line_numbers: bool,
    pub(crate) whitespace: bool,
    pub(crate) soft_wrap: bool,
    pub(crate) status_role: StatusRole,
    pub(crate) status_theme: StatusTheme,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            cursor_shape: CursorShape::Default,
            highlight: None,
            syntax: SyntaxKind::Plain,
            line_numbers: false,
            whitespace: false,
            soft_wrap: false,
            status_role: StatusRole::Normal,
            status_theme: StatusTheme::default(),
        }
    }
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
/// Bottom row (height) is reserved for the semantic status bar; content uses height-1.
/// Horizontal slicing starts at a scalar document column but clips by terminal cells.
/// Status text is pinned by absolute positioning and styled through `RenderOptions`.
pub fn render_buffer<W: Write + ?Sized>(
    out: &mut W,
    buffer: &dyn Buffer,
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions,
) -> io::Result<()> {
    let mut frame = Vec::new();
    if options.soft_wrap {
        wrapped::compose_buffer(&mut frame, buffer, viewport, message, options)?;
    } else {
        compose_buffer(&mut frame, buffer, viewport, message, options)?;
    }
    out.write_all(&frame)?;
    out.flush()
}

fn compose_buffer(
    out: &mut Vec<u8>,
    buffer: &dyn Buffer,
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions,
) -> io::Result<()> {
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

    if height > 0 {
        status_bar::write_status_bar(
            out,
            height,
            width,
            message.unwrap_or(""),
            options.status_role,
            options.status_theme,
        )?;
    }

    let position = unwrapped_cursor_position(buffer, cursor, &visible, viewport, gutter);
    write_terminal_cursor(out, position, options.cursor_shape)
}

fn unwrapped_cursor_position(
    buffer: &dyn Buffer,
    cursor: Cursor,
    visible: &[crate::buffer::LineView],
    viewport: RenderViewport,
    gutter: usize,
) -> Option<(usize, usize)> {
    let content_h = viewport.height.saturating_sub(1);
    let content_w = viewport.width.saturating_sub(gutter);
    let Cursor { row, col } = cursor;
    let start_row = viewport.start_row;
    let start_col = viewport.start_col;
    let row_visible = row >= start_row && row < start_row.saturating_add(content_h);
    let cursor_cells = if row_visible && col >= start_col {
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
    let line_end = buffer.line_char_count(row).unwrap_or(0);
    let col_visible = col >= start_col
        && (cursor_cells < content_w || (col == line_end && cursor_cells == content_w));
    (row_visible && col_visible && content_w > 0).then(|| {
        (
            row - start_row + 1,
            gutter
                .saturating_add(cursor_cells)
                .saturating_add(1)
                .min(viewport.width.max(1)),
        )
    })
}

pub(super) fn write_terminal_cursor(
    out: &mut Vec<u8>,
    position: Option<(usize, usize)>,
    shape: CursorShape,
) -> io::Result<()> {
    cursor_style::write_shape(out, shape)?;
    match position {
        Some((row, col)) => write!(out, "\x1b[{row};{col}H\x1b[?25h"),
        None => write!(out, "\x1b[?25l\x1b[1;1H"),
    }
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
    fn source_buffer_terminal_controls_render_inertly() {
        let b = SimpleBuffer::from_text(
            "visible-before\x1b[2JCONTROL-CLEAR\x1b]52;c;cGF5bG9hZA==\x07visible-after\u{009b}?2004h",
        );
        let mut out = Vec::new();

        render_buffer(
            &mut out,
            &b,
            RenderViewport::new(0, 0, 3, 120),
            None,
            RenderOptions::default(),
        )
        .unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(!rendered.contains("\x1b[2J"));
        assert!(!rendered.contains("\x1b]52"));
        assert!(!rendered.contains('\x07'));
        assert!(!rendered.contains('\u{009b}'));
        assert!(rendered.contains("visible-before␛[2JCONTROL-CLEAR"));
        assert!(rendered.contains("␛]52;c;cGF5bG9hZA==␇visible-after�?2004h"));
    }

    #[test]
    fn markdown_preview_raw_html_terminal_controls_render_inertly() {
        let preview =
            crate::editor::markdown_preview::render("<span>before\x1b[2Jafter\x07</span>");
        let b = SimpleBuffer::from_text(&preview);
        let mut out = Vec::new();

        render_buffer(
            &mut out,
            &b,
            RenderViewport::new(0, 0, 3, 80),
            None,
            RenderOptions {
                syntax: SyntaxKind::MarkdownPreview,
                ..RenderOptions::default()
            },
        )
        .unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(!rendered.contains("\x1b[2J"));
        assert!(!rendered.contains('\x07'));
        assert!(rendered.contains("<span>before␛[2Jafter␇</span>"));
    }

    #[test]
    fn markdown_styles_preserve_line_number_tab_selection_and_cursor_cells() {
        let mut buffer = SimpleBuffer::from_text("\t**猫** | [x](u)");
        buffer.set_cursor(Cursor { row: 0, col: 3 });
        let mut out = Vec::new();

        render_buffer(
            &mut out,
            &buffer,
            RenderViewport::new(0, 0, 3, 24),
            None,
            RenderOptions {
                highlight: Some(TextHighlight {
                    start: Cursor { row: 0, col: 3 },
                    end: Cursor { row: 0, col: 4 },
                }),
                syntax: SyntaxKind::Markdown,
                line_numbers: true,
                ..RenderOptions::default()
            },
        )
        .unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("\x1b[2;90m1 \x1b[0m    \x1b[3;35m**\x1b[0m"));
        assert!(rendered.contains("\x1b[3;35;7m猫\x1b[0m"));
        assert!(rendered.ends_with("\x1b[1;9H\x1b[?25h"));
    }

    #[test]
    fn wrapped_command_preview_terminal_controls_render_inertly() {
        let b = SimpleBuffer::from_text("preview-before\x1b[2Jpreview-after\x07");
        let mut out = Vec::new();

        render_buffer(
            &mut out,
            &b,
            RenderViewport::new(0, 0, 4, 80),
            Some("Command output (read-only)."),
            RenderOptions {
                soft_wrap: true,
                ..RenderOptions::default()
            },
        )
        .unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(!rendered.contains("\x1b[2J"));
        assert!(!rendered.contains('\x07'));
        assert!(rendered.contains("preview-before␛[2Jpreview-after␇"));
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
        assert!(s.contains("\x1b[4;1H"));
        assert!(s.contains("\x1b[2Kstatus"));
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
        assert!(rendered.ends_with("\x1b[1;4H\x1b[?25h"));
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
