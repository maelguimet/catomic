//! Purpose: verify ghost layout across cursor suffixes, Unicode cells, wrapping, and scrolling.
//! Owns: pure captured-frame fixtures over SimpleBuffer.
//! Must not: mutate buffers, start model work, or depend on a real terminal.
//! Invariants: ghost ANSI is dim gray and the final terminal cursor stays at document cursor.

use super::*;
use crate::buffer::{Cursor, SimpleBuffer};

fn render(
    text: &str,
    cursor: Cursor,
    ghost: &str,
    viewport: RenderViewport,
    options: RenderOptions,
) -> (SimpleBuffer, String) {
    let mut buffer = SimpleBuffer::from_text(text);
    buffer.set_cursor(cursor);
    let mut out = Vec::new();
    super::super::render_buffer_with_ghost(
        &mut out,
        &buffer,
        viewport,
        Some("ready"),
        options,
        Some(GhostText {
            cursor,
            text: ghost,
        }),
    )
    .unwrap();
    (buffer, String::from_utf8(out).unwrap())
}

#[test]
fn inline_ghost_shifts_suffix_without_changing_buffer_or_cursor() {
    let cursor = Cursor { row: 0, col: 5 };
    let (buffer, frame) = render(
        "Hello world",
        cursor,
        " brave",
        RenderViewport::new(0, 0, 3, 40),
        RenderOptions::default(),
    );

    assert!(frame.contains("Hello\x1b[90;2m brave\x1b[0m world"));
    assert!(frame.ends_with("\x1b[1;6H\x1b[?25h\x1b[?2026l"));
    assert_eq!(buffer.to_string(), "Hello world");
    assert_eq!(buffer.cursor(), cursor);
}

#[test]
fn multiline_wide_ghost_uses_virtual_rows_and_keeps_real_suffix() {
    let (_, frame) = render(
        "hello!\nsource row",
        Cursor { row: 0, col: 5 },
        "\n猫🙂",
        RenderViewport::new(0, 0, 5, 20),
        RenderOptions {
            line_numbers: true,
            ..RenderOptions::default()
        },
    );

    assert!(frame.contains("1 \x1b[0mhello"), "frame: {frame:?}");
    assert!(frame.contains("\x1b[90;2m猫🙂\x1b[0m!"));
    assert!(frame.contains("2 \x1b[0msource row"));
    assert!(frame.ends_with("\x1b[1;8H\x1b[?25h\x1b[?2026l"));
}

#[test]
fn tabs_inside_ghost_expand_from_the_real_prefix_cell() {
    let (_, frame) = render(
        "a",
        Cursor { row: 0, col: 1 },
        "\tb",
        RenderViewport::new(0, 0, 3, 20),
        RenderOptions::default(),
    );

    assert!(frame.contains("a\x1b[90;2m   b\x1b[0m"));
}

#[test]
fn ghost_style_overrides_syntax_without_disabling_source_highlighting() {
    let (_, frame) = render(
        "let value = 1;",
        Cursor { row: 0, col: 4 },
        "mut ",
        RenderViewport::new(0, 0, 3, 40),
        RenderOptions {
            syntax: crate::editor::syntax::SyntaxKind::Rust,
            ..RenderOptions::default()
        },
    );

    assert!(frame.contains("\x1b[35mlet\x1b[0m"));
    assert!(frame.contains("\x1b[90;2mmut\x1b[0m"));
    assert!(frame.contains("value"));
    assert!(frame.contains("\x1b[33m1\x1b[0m"));
}

#[test]
fn horizontal_scroll_and_narrow_width_clip_ghost_on_grapheme_boundaries() {
    let (_, frame) = render(
        "0123a\u{301}Z",
        Cursor { row: 0, col: 6 },
        "猫🙂x",
        RenderViewport::new(0, 4, 3, 4),
        RenderOptions::default(),
    );

    assert!(frame.contains("a\u{301}\x1b[90;2m猫\x1b[0m"));
    assert!(!frame.contains('🙂'));
    assert!(frame.ends_with("\x1b[1;2H\x1b[?25h\x1b[?2026l"));
}

#[test]
fn soft_wrap_splits_wide_ghost_without_moving_cursor() {
    let (_, frame) = render(
        "abZ",
        Cursor { row: 0, col: 2 },
        "猫🙂x",
        RenderViewport::new(0, 0, 5, 5),
        RenderOptions {
            soft_wrap: true,
            ..RenderOptions::default()
        },
    );

    assert!(frame.contains("\x1b[1;1H\x1b[Kab\x1b[90;2m猫\x1b[0m"));
    assert!(frame.contains("\x1b[2;1H\x1b[K\x1b[90;2m🙂x\x1b[0mZ"));
    assert!(frame.ends_with("\x1b[1;3H\x1b[?25h\x1b[?2026l"));
}
