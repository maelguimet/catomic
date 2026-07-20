//! Purpose: verify coherent frame publication and final terminal-cell cursor coordinates.
//! Owns: ANSI frame-envelope, back-to-back ordering, and mixed-layout cursor fixtures.
//! Must not: require a real terminal, mutate through rendering, access disk, or start tasks.
//! Invariants: every complete frame is synchronized and its final caret matches rendered cells.
//! Phase: issue #128 terminal flicker and visual cursor drift regression coverage.

use crate::buffer::{Buffer, Cursor, SimpleBuffer};
use crate::editor::syntax::SyntaxKind;

use super::*;

#[test]
fn frame_hides_the_caret_and_publishes_one_synchronized_update() {
    let frame = render(
        "# heading",
        Cursor { row: 0, col: 1 },
        RenderViewport::new(0, 0, 3, 20),
        RenderOptions {
            syntax: SyntaxKind::Markdown,
            ..RenderOptions::default()
        },
    );

    assert!(frame.starts_with("\x1b[?2026h\x1b[?25l"));
    assert_eq!(frame.matches("\x1b[?2026h").count(), 1);
    assert_eq!(frame.matches("\x1b[?2026l").count(), 1);
    assert!(position(&frame, "\x1b[?25l") < position(&frame, "\x1b[1;1H"));
    assert!(position(&frame, "\x1b[?25h") < position(&frame, "\x1b[?2026l"));
    assert_final_cursor(&frame, 1, 2);
}

#[test]
fn final_cursor_uses_cells_across_unicode_tabs_styles_gutters_and_scrolling() {
    let fixtures = [
        ("\t#", Cursor { row: 0, col: 2 }, 0, 6),
        ("e\u{301}#", Cursor { row: 0, col: 3 }, 0, 3),
        ("🙂#", Cursor { row: 0, col: 2 }, 0, 4),
        ("0123\t#x", Cursor { row: 0, col: 6 }, 4, 6),
    ];
    for (text, cursor, start_col, terminal_col) in fixtures {
        let frame = render(
            text,
            cursor,
            RenderViewport::new(0, start_col, 3, 12),
            RenderOptions {
                syntax: SyntaxKind::Markdown,
                ..RenderOptions::default()
            },
        );
        assert_final_cursor(&frame, 1, terminal_col);
    }

    let ranges = [TextHighlight {
        start: Cursor { row: 0, col: 0 },
        end: Cursor { row: 0, col: 1 },
    }];
    let gutter_lines = [0];
    let frame = render(
        "#",
        Cursor { row: 0, col: 1 },
        RenderViewport::new(0, 0, 3, 12),
        RenderOptions {
            syntax: SyntaxKind::Markdown,
            line_numbers: true,
            llm_changes: Some(LlmChanges {
                ranges: &ranges,
                gutter_lines: &gutter_lines,
            }),
            ..RenderOptions::default()
        },
    );
    assert_final_cursor(&frame, 1, 6);
}

#[test]
fn soft_wrap_places_a_boundary_cursor_on_the_continuation_row() {
    let frame = render(
        "abc#def",
        Cursor { row: 0, col: 4 },
        RenderViewport::new(0, 0, 4, 4),
        RenderOptions {
            syntax: SyntaxKind::Markdown,
            soft_wrap: true,
            ..RenderOptions::default()
        },
    );

    assert_final_cursor(&frame, 2, 1);
}

#[test]
fn back_to_back_frames_finish_before_the_newer_state_begins() {
    let mut stream = Vec::new();
    let mut old = SimpleBuffer::from_text("# old");
    old.set_cursor(Cursor { row: 0, col: 1 });
    render_buffer(
        &mut stream,
        &old,
        RenderViewport::new(0, 0, 3, 20),
        None,
        RenderOptions::default(),
    )
    .unwrap();
    let boundary = stream.len();

    let mut newer = SimpleBuffer::from_text("\t# new");
    newer.set_cursor(Cursor { row: 0, col: 2 });
    render_buffer(
        &mut stream,
        &newer,
        RenderViewport::new(0, 0, 3, 20),
        None,
        RenderOptions::default(),
    )
    .unwrap();

    let old_frame = std::str::from_utf8(&stream[..boundary]).unwrap();
    let newer_frame = std::str::from_utf8(&stream[boundary..]).unwrap();
    assert!(old_frame.ends_with("\x1b[?2026l"));
    assert!(newer_frame.starts_with("\x1b[?2026h\x1b[?25l"));
    assert_final_cursor(newer_frame, 1, 6);
}

fn render(
    text: &str,
    cursor: Cursor,
    viewport: RenderViewport,
    options: RenderOptions<'_>,
) -> String {
    let mut buffer = SimpleBuffer::from_text(text);
    buffer.set_cursor(cursor);
    let mut out = Vec::new();
    render_buffer(&mut out, &buffer, viewport, None, options).unwrap();
    String::from_utf8(out).unwrap()
}

fn assert_final_cursor(frame: &str, row: usize, col: usize) {
    assert!(
        frame.ends_with(&format!("\x1b[0 q\x1b[{row};{col}H\x1b[?25h\x1b[?2026l")),
        "unexpected final cursor for frame {frame:?}"
    );
}

fn position(haystack: &str, needle: &str) -> usize {
    haystack.find(needle).expect("terminal sequence")
}
