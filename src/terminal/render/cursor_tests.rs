//! Purpose: verify safe terminal-caret behavior for viewport-only scrolling.
//! Owns: unwrapped vertical and horizontal off-screen cursor render fixtures.
//! Must not: mutate through rendering, require a terminal, access disk, or emit network work.
//! Invariants: an off-screen document cursor is hidden and never placed on the status row.
//! Phase: post-v0.1 viewport-only wheel scrolling.

use crate::buffer::{Buffer, Cursor, SimpleBuffer};

use super::*;

#[test]
fn render_hides_an_offscreen_cursor_at_a_safe_content_position() {
    let mut buffer = SimpleBuffer::from_text("zero\none\ntwo\nthree\nfour");
    buffer.set_cursor(Cursor { row: 0, col: 2 });
    let mut out = Vec::new();

    render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(2, 0, 4, 10),
        Some("status"),
        RenderOptions::default(),
    )
    .unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.ends_with("\x1b[?25l\x1b[1;1H"));
    assert!(!rendered.ends_with("\x1b[4;3H"));
}

#[test]
fn render_hides_a_cursor_left_of_the_horizontal_viewport() {
    let mut buffer = SimpleBuffer::from_text("0123456789");
    buffer.set_cursor(Cursor { row: 0, col: 1 });
    let mut out = Vec::new();

    render_buffer(
        &mut out,
        &buffer,
        RenderViewport::new(0, 4, 3, 5),
        None,
        RenderOptions::default(),
    )
    .unwrap();

    assert!(String::from_utf8(out)
        .unwrap()
        .ends_with("\x1b[?25l\x1b[1;1H"));
}
