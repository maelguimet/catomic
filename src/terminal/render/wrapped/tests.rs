//! Purpose: verify bounded soft-wrap row splitting, cursor mapping, and rendering.
//! Owns: focused ASCII, Unicode, gutter, and viewport-start fixtures.
//! Must not: mutate App state, access disk/network, or require a real terminal.
//! Invariants: visual rows preserve source text and never split grapheme clusters.
//! Phase: post-v0.1 core usability.

use crate::buffer::{Buffer, Cursor, SimpleBuffer};
use crate::config::theme::{Color, Style, Theme};
use crate::editor::syntax::SyntaxKind;

use super::*;

#[test]
fn visible_rows_wrap_without_changing_document_coordinates() {
    let buffer = SimpleBuffer::from_text("abcdef\n猫猫x");
    let rows = visible_rows(&buffer, 0, 0, 4, 3).unwrap();
    assert_eq!(rows.len(), 4);
    assert_eq!((rows[0].document_row, rows[0].start_col), (0, 0));
    assert_eq!(rows[0].content, "abc");
    assert_eq!((rows[1].document_row, rows[1].start_col), (0, 3));
    assert_eq!(rows[1].content, "def");
    assert_eq!((rows[2].document_row, rows[2].start_col), (1, 0));
    assert_eq!(rows[2].content, "猫");
    assert_eq!((rows[3].document_row, rows[3].start_col), (1, 1));
    assert_eq!(rows[3].content, "猫x");
}

#[test]
fn wrapped_cursor_uses_the_continuation_row_and_cell_width() {
    let mut buffer = SimpleBuffer::from_text("ab猫x");
    buffer.set_cursor(Cursor { row: 0, col: 3 });
    let rows = visible_rows(&buffer, 0, 0, 3, 3).unwrap();
    assert_eq!(
        wrapped_cursor_position(buffer.cursor(), &rows, 0, 3),
        Some((2, 3))
    );
    assert!(cursor_is_visible(&buffer, 0, 0, 3, 3).unwrap());
}

#[test]
fn wrapped_render_hides_a_document_cursor_above_the_viewport() {
    let mut buffer = SimpleBuffer::from_text("abcdef\nnext");
    buffer.set_cursor(Cursor { row: 0, col: 0 });
    let mut out = Vec::new();

    super::super::render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 3, 3).with_wrap_col(3),
        None,
        RenderOptions {
            soft_wrap: true,
            ..RenderOptions::default()
        },
    )
    .unwrap();

    assert!(String::from_utf8(out)
        .unwrap()
        .ends_with("\x1b[?25l\x1b[1;1H"));
}

#[test]
fn wrapped_render_emits_each_visual_row() {
    let buffer = SimpleBuffer::from_text("abcdef");
    let mut out = Vec::new();
    super::super::render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 4, 3),
        Some("go"),
        RenderOptions {
            soft_wrap: true,
            status_role: super::super::StatusRole::Prompt,
            status_theme: super::super::StatusTheme::monochrome(),
            ..RenderOptions::default()
        },
    )
    .unwrap();
    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("\x1b[1;1H\x1b[Kabc"));
    assert!(rendered.contains("\x1b[2;1H\x1b[Kdef"));
    assert!(rendered.contains("\x1b[4;1H\x1b[4m\x1b[7m\x1b[2Kgo \x1b[0m"));
}

#[test]
fn markdown_styles_do_not_change_soft_wrap_coordinates() {
    let source = "**bold** | 猫";
    let mut buffer = SimpleBuffer::from_text(source);
    buffer.set_cursor(Cursor { row: 0, col: 8 });
    let rows = visible_rows(&buffer, 0, 0, 3, 8).unwrap();

    assert_eq!(rows[0].content, "**bold**");
    assert_eq!(rows[1].start_col, 8);
    assert_eq!(rows[1].content, " | 猫");
    assert_eq!(
        wrapped_cursor_position(buffer.cursor(), &rows, 0, 8),
        Some((2, 1))
    );

    let mut out = Vec::new();
    super::super::render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 4, 8),
        None,
        RenderOptions {
            syntax: SyntaxKind::Markdown,
            soft_wrap: true,
            ..RenderOptions::default()
        },
    )
    .unwrap();
    assert!(String::from_utf8(out)
        .unwrap()
        .contains("\x1b[35m**bold**\x1b[0m"));
}

#[test]
fn wrapped_continuation_gutter_inherits_the_base_background() {
    let buffer = SimpleBuffer::from_text("abcdef");
    let mut out = Vec::new();
    let theme = Theme {
        text: Style::pair(Color::Ansi(7), Color::Ansi(0)),
        line_number: Style::fg(Color::Ansi(6)),
        ..Theme::default()
    };
    super::super::render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 4, 6),
        None,
        RenderOptions {
            soft_wrap: true,
            line_numbers: true,
            theme,
            ..RenderOptions::default()
        },
    )
    .unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("\x1b[2;1H\x1b[37;40m\x1b[K\x1b[0m\x1b[36;40m  \x1b[0m"));
}
