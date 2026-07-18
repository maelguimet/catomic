//! Purpose: render the visible buffer viewport as complete ANSI frames.
//! Owns: frame composition, row clearing, visible styled text, bottom annotation, and cursor placement.
//! Must not: mutate editor/buffer state, read full buffers, or own terminal setup.
//! Invariants: file-backed read errors propagate before output; every viewport row is cleared;
//!   a frame is committed in one write; rendering never emits a full-screen clear.
//! Phase: 4-a viewport-only syntax styling.

use std::io::{self, Write};

use crate::buffer::{Buffer, Cursor};
use crate::config::theme::Theme;
use crate::editor::syntax::SyntaxKind;
use crate::terminal::cursor_style::{self, CursorShape};

#[cfg(test)]
mod cursor_tests;
mod frame;
mod ghost;
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
pub(crate) struct GhostText<'a> {
    pub(crate) cursor: Cursor,
    pub(crate) text: &'a str,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum HighlightKind {
    #[default]
    Selection,
    Search,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ContentSurface {
    #[default]
    Normal,
    Preview,
    Diff,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LlmChanges<'a> {
    pub(crate) ranges: &'a [TextHighlight],
    pub(crate) gutter_lines: &'a [usize],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RenderOptions<'a> {
    pub(crate) cursor_shape: CursorShape,
    pub(crate) highlight: Option<TextHighlight>,
    pub(crate) highlight_kind: HighlightKind,
    pub(crate) llm_changes: Option<LlmChanges<'a>>,
    pub(crate) syntax: SyntaxKind,
    pub(crate) surface: ContentSurface,
    pub(crate) theme: Theme,
    pub(crate) line_numbers: bool,
    pub(crate) whitespace: bool,
    pub(crate) soft_wrap: bool,
    pub(crate) status_role: StatusRole,
    pub(crate) status_theme: StatusTheme,
}

impl Default for RenderOptions<'_> {
    fn default() -> Self {
        Self {
            cursor_shape: CursorShape::Default,
            highlight: None,
            highlight_kind: HighlightKind::Selection,
            llm_changes: None,
            syntax: SyntaxKind::Plain,
            surface: ContentSurface::Normal,
            theme: Theme::default(),
            line_numbers: false,
            whitespace: false,
            soft_wrap: false,
            status_role: StatusRole::Normal,
            status_theme: StatusTheme::default(),
        }
    }
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

pub(crate) fn change_gutter_width(has_changes: bool) -> usize {
    usize::from(has_changes) * 2
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
    options: RenderOptions<'_>,
) -> io::Result<()> {
    let mut frame = Vec::new();
    style::write_cursor_color(&mut frame, options.theme)?;
    if options.soft_wrap {
        wrapped::compose_buffer(&mut frame, buffer, viewport, message, options)?;
    } else {
        frame::compose_buffer(&mut frame, buffer, viewport, message, options)?;
    }
    out.write_all(&frame)?;
    out.flush()
}

pub(crate) fn render_buffer_with_ghost<W: Write + ?Sized>(
    out: &mut W,
    buffer: &dyn Buffer,
    viewport: RenderViewport,
    message: Option<&str>,
    options: RenderOptions<'_>,
    ghost: Option<GhostText<'_>>,
) -> io::Result<()> {
    let Some(ghost) = ghost.filter(|ghost| ghost.cursor == buffer.cursor()) else {
        return render_buffer(out, buffer, viewport, message, options);
    };
    let mut frame = Vec::new();
    style::write_cursor_color(&mut frame, options.theme)?;
    ghost::compose_buffer(&mut frame, buffer, viewport, message, options, ghost)?;
    out.write_all(&frame)?;
    out.flush()
}

pub(super) fn write_line_number<W: Write + ?Sized>(
    out: &mut W,
    row: usize,
    gutter: usize,
    theme: Theme,
) -> std::io::Result<()> {
    let label = format!(
        "{:>width$} ",
        row.saturating_add(1),
        width = gutter.saturating_sub(1)
    );
    let clipped: String = label.chars().take(gutter).collect();
    style::write_styled_text(
        out,
        &clipped,
        theme.text.overlay(theme.line_number),
        theme.truecolor,
    )
}

pub(super) fn write_change_gutter<W: Write + ?Sized>(
    out: &mut W,
    row: usize,
    changes: Option<LlmChanges<'_>>,
    theme: Theme,
) -> std::io::Result<()> {
    let Some(changes) = changes else {
        return write!(out, "  ");
    };
    if changes.gutter_lines.contains(&row) {
        style::write_semantic_gutter(out, theme.llm_changed, theme.truecolor)
    } else {
        write!(out, "  ")
    }
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
                highlight_kind: HighlightKind::Search,
                ..RenderOptions::default()
            },
        )
        .unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(rendered.contains("zero \x1b[30;43mtarget\x1b[0m here"));
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
        assert!(rendered.contains("zero \x1b[30;46mhere\x1b[0m"));
        assert!(rendered.contains("\x1b[30;46mmiddle\x1b[0m"));
        assert!(rendered.contains("\x1b[30;46mlast\x1b[0m row"));
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
    fn status_terminal_controls_render_inertly() {
        let b = SimpleBuffer::from_text("");
        let mut out = Vec::new();

        render_buffer(
            &mut out,
            &b,
            RenderViewport::new(0, 0, 2, 80),
            Some("error from hostile\x1b]0;title\x07path"),
            RenderOptions::default(),
        )
        .unwrap();

        let rendered = String::from_utf8(out).unwrap();
        assert!(!rendered.contains("\x1b]0"));
        assert!(!rendered.contains('\x07'));
        assert!(rendered.contains("error from hostile␛]0;title␇path"));
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
        assert!(s.contains("\x1b[4;1H\x1b[30m\x1b[47m\x1b[2Kstatus"));
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
        assert!(rendered.ends_with("\x1b[0 q\x1b[1;4H\x1b[?25h"));
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
