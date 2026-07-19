//! Purpose: characterize terminal render transport and composed-frame behavior.
//! Owns: highlight, safety, viewport, Unicode, and file-backed render regressions.
//! Must not: mutate production state, require a real terminal, or duplicate transport tests.
//! Invariants: hostile controls stay inert; viewport and Unicode boundaries remain stable.
//! Phase: bounded post-beta render ownership cleanup.

use super::*;
use crate::buffer::SimpleBuffer;

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
    render_buffer(
        &mut out,
        &b,
        RenderViewport::new(0, 0, 0, 10),
        None,
        RenderOptions::default(),
    )
    .expect("render h=0");
    let s = String::from_utf8_lossy(&out);
    assert!(
        !s.contains("\x1b[0;"),
        "height=0 must not emit bottom-row positioning: {}",
        s
    );
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
    assert!(
        !s.contains("L0") && !s.contains("L1") && !s.contains("L2"),
        "height=1 must emit no content lines: {}",
        s
    );
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
    assert!(
        !s.contains("abc") && !s.contains("def"),
        "width=0 must emit no line content chars: {}",
        s
    );
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
    assert!(s.contains("\x1b[4;1H\x1b[2K\x1b[0m\x1b[90m\x1b[2mstatus"));
}

#[test]
fn mobile_action_bar_reserves_a_second_bottom_row_and_clips_both_rows() {
    let b = SimpleBuffer::from_text("one\ntwo\nthree");
    let mut out = Vec::new();
    render_buffer(
        &mut out,
        &b,
        RenderViewport::new(0, 0, 4, 10),
        Some("status that wraps"),
        RenderOptions {
            action_bar: Some("[Menu][Save][Undo]"),
            ..RenderOptions::default()
        },
    )
    .unwrap();

    let rendered = String::from_utf8(out).unwrap();
    assert!(rendered.contains("\x1b[1;1H\x1b[Kone"));
    assert!(rendered.contains("\x1b[2;1H\x1b[Ktwo"));
    assert!(!rendered.contains("three"));
    assert!(rendered.contains("\x1b[3;1H"));
    assert!(rendered.contains("statu…raps"), "{rendered:?}");
    assert!(rendered.contains("\x1b[4;1H"));
    assert!(rendered.contains("[Menu…ndo]"));
    assert!(!rendered.contains("status that wraps"));
}

#[test]
fn render_buffer_start_col_zero_nonzero_width_preserves_default_visible_output() {
    let b = SimpleBuffer::from_text("0123456789\nABCDEFGHIJ\n");
    let mut out_default: Vec<u8> = Vec::new();
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

    assert_eq!(default_s, explicit_s);
    assert!(
        default_s.contains("012345"),
        "first 6 chars of first line visible with start_col=0"
    );
}

#[test]
fn render_buffer_horizontal_cell_clipping_preserves_grapheme_boundaries() {
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
