//! Purpose: verify bounded soft-wrap row splitting, cursor mapping, and rendering.
//! Owns: focused ASCII, Unicode, gutter, and viewport-start fixtures.
//! Must not: mutate App state, access disk/network, or require a real terminal.
//! Invariants: visual rows preserve source text and never split grapheme clusters.
//! Phase: post-v0.1 core usability.

use crate::buffer::{Buffer, Cursor, SimpleBuffer};

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
    assert_eq!(wrapped_cursor_position(buffer.cursor(), &rows, 0), (2, 3));
    assert!(cursor_is_visible(&buffer, 0, 0, 3, 3).unwrap());
}

#[test]
fn wrapped_render_emits_each_visual_row() {
    let buffer = SimpleBuffer::from_text("abcdef");
    let mut out = Vec::new();
    render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 0, 4, 3),
        None,
        RenderOptions {
            soft_wrap: true,
            ..RenderOptions::default()
        },
    )
    .unwrap();
    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("\x1b[1;1H\x1b[Kabc"));
    assert!(rendered.contains("\x1b[2;1H\x1b[Kdef"));
}
