//! Purpose: verify keyboard selection, clipboard, and selection-aware App edits.
//! Owns: captured key/render fixtures and bracketed-paste integration assertions.
//! Must not: require a real terminal, system clipboard reader, mouse, or filesystem.
//! Invariants: selection replacement and paste each undo as one transaction.

use super::*;
use crossterm::event::{KeyEventKind, KeyEventState};

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: KeyEventState::NONE,
    }
}

fn app_with(text: &str) -> super::super::App {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(crate::buffer::PieceTable::from_text(text));
    app
}

fn send(app: &mut super::super::App, out: &mut Vec<u8>, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_key_with(out, key(code, modifiers)).unwrap();
}

fn send_control(app: &mut super::super::App, out: &mut Vec<u8>, character: char) {
    send(app, out, KeyCode::Char(character), KeyModifiers::CONTROL);
}

#[test]
fn shift_arrows_populate_both_clipboards_before_ctrl_c() {
    let mut app = app_with("abc");
    let mut out = Vec::new();

    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::SHIFT);
    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::SHIFT);
    assert_eq!(app.clipboard, "ab");
    assert!(String::from_utf8_lossy(&out).contains("\x1b]52;c;YWI=\x1b\\"));

    out.clear();
    send(
        &mut app,
        &mut out,
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    );

    assert_eq!(app.clipboard, "ab");
    assert_eq!(
        app.selection.active().unwrap().ordered(),
        (Cursor { row: 0, col: 0 }, Cursor { row: 0, col: 2 })
    );
    assert!(String::from_utf8_lossy(&out).contains("\x1b]52;c;YWI=\x1b\\"));
}

#[test]
fn alt_shift_arrows_select_by_word_when_terminal_owns_ctrl_shift_arrows() {
    let mut app = app_with("one two three");
    let mut out = Vec::new();
    let fallback = KeyModifiers::ALT | KeyModifiers::SHIFT;

    send(&mut app, &mut out, KeyCode::Right, fallback);
    assert_eq!(
        app.selection.active().unwrap().ordered(),
        (Cursor { row: 0, col: 0 }, Cursor { row: 0, col: 4 })
    );

    send(&mut app, &mut out, KeyCode::Right, fallback);
    send(&mut app, &mut out, KeyCode::Left, fallback);
    assert_eq!(
        app.selection.active().unwrap().ordered(),
        (Cursor { row: 0, col: 0 }, Cursor { row: 0, col: 4 })
    );
}

#[test]
fn cut_and_internal_paste_are_single_undoable_edits() {
    let mut app = app_with("abc");
    let mut out = Vec::new();
    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::SHIFT);
    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::SHIFT);

    send(
        &mut app,
        &mut out,
        KeyCode::Char('x'),
        KeyModifiers::CONTROL,
    );
    assert_eq!(app.buffer.to_string(), "c");
    assert_eq!(app.clipboard, "ab");
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "abc");
    app.buffer.redo();
    assert_eq!(app.buffer.to_string(), "c");

    send(
        &mut app,
        &mut out,
        KeyCode::Char('v'),
        KeyModifiers::CONTROL,
    );
    assert_eq!(app.buffer.to_string(), "abc");
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "c");
}

#[test]
fn ctrl_k_cuts_whole_lines_and_consecutive_cuts_append_in_order() {
    let mut app = app_with("one\ntwo\nthree");
    let mut out = Vec::new();

    send_control(&mut app, &mut out, 'k');
    assert_eq!(app.buffer.to_string(), "two\nthree");
    assert_eq!(app.clipboard, "one\n");
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 0 });

    send_control(&mut app, &mut out, 'k');
    assert_eq!(app.buffer.to_string(), "three");
    assert_eq!(app.clipboard, "one\ntwo\n");
    assert!(String::from_utf8_lossy(&out).contains("\x1b]52;c;b25lCnR3bwo=\x1b\\"));

    send_control(&mut app, &mut out, 'v');
    assert_eq!(app.buffer.to_string(), "one\ntwo\nthree");
}

#[test]
fn ctrl_k_handles_empty_final_and_no_final_newline_lines() {
    let cases = [
        ("first\nlast", Cursor { row: 0, col: 3 }, "last", "first\n"),
        ("first\nlast", Cursor { row: 1, col: 2 }, "first\n", "last"),
        ("solo", Cursor { row: 0, col: 2 }, "", "solo"),
        (
            "first\n\nlast",
            Cursor { row: 1, col: 0 },
            "first\nlast",
            "\n",
        ),
    ];
    for (source, cursor, expected, clipboard) in cases {
        let mut app = app_with(source);
        let mut out = Vec::new();
        app.buffer.set_cursor(cursor);
        send_control(&mut app, &mut out, 'k');
        assert_eq!(app.buffer.to_string(), expected, "source {source:?}");
        assert_eq!(app.clipboard, clipboard, "source {source:?}");
    }

    for source in ["", "first\n"] {
        let mut app = app_with(source);
        let mut out = Vec::new();
        if source.ends_with('\n') {
            app.buffer.set_cursor(Cursor { row: 1, col: 0 });
        }
        send_control(&mut app, &mut out, 'k');
        assert_eq!(app.buffer.to_string(), source);
        assert!(app.clipboard.is_empty());
        assert_eq!(app.message.as_deref(), Some("Nothing to cut on this line."));
    }
}

#[test]
fn ctrl_k_preserves_selection_cut_and_unicode_bytes() {
    let mut selected = app_with("abc\nrest");
    let mut out = Vec::new();
    send(&mut selected, &mut out, KeyCode::Right, KeyModifiers::SHIFT);
    send(&mut selected, &mut out, KeyCode::Right, KeyModifiers::SHIFT);
    send_control(&mut selected, &mut out, 'k');
    assert_eq!(selected.buffer.to_string(), "c\nrest");
    assert_eq!(selected.clipboard, "ab");

    let mut app = app_with("e\u{301}猫🙂\nnext");
    let mut out = Vec::new();
    app.buffer.set_cursor(Cursor { row: 0, col: 2 });
    send_control(&mut app, &mut out, 'k');
    assert_eq!(app.clipboard, "e\u{301}猫🙂\n");
    assert_eq!(app.buffer.to_string(), "next");
}

#[test]
fn repeated_line_cuts_are_independent_undo_transactions() {
    let mut app = app_with("a\nb\nc");
    let mut out = Vec::new();
    for _ in 0..2 {
        send_control(&mut app, &mut out, 'k');
    }
    assert_eq!(app.buffer.to_string(), "c");
    assert_eq!(app.clipboard, "a\nb\n");

    send_control(&mut app, &mut out, 'z');
    assert_eq!(app.buffer.to_string(), "b\nc");
    send_control(&mut app, &mut out, 'z');
    assert_eq!(app.buffer.to_string(), "a\nb\nc");
    send_control(&mut app, &mut out, 'y');
    assert_eq!(app.buffer.to_string(), "b\nc");
    send_control(&mut app, &mut out, 'y');
    assert_eq!(app.buffer.to_string(), "c");
}

#[test]
fn movement_editing_and_buffer_switches_end_the_cut_line_chain() {
    let mut moved = app_with("a\nb\nc");
    let mut out = Vec::new();
    send_control(&mut moved, &mut out, 'k');
    send(&mut moved, &mut out, KeyCode::Down, KeyModifiers::NONE);
    send_control(&mut moved, &mut out, 'k');
    assert_eq!(moved.clipboard, "c");

    let mut edited = app_with("a\nb\nc");
    let mut out = Vec::new();
    send_control(&mut edited, &mut out, 'k');
    send(
        &mut edited,
        &mut out,
        KeyCode::Char('X'),
        KeyModifiers::SHIFT,
    );
    send_control(&mut edited, &mut out, 'k');
    assert_eq!(edited.clipboard, "Xb\n");

    let mut switched = app_with("left\nrest");
    let mut out = Vec::new();
    send_control(&mut switched, &mut out, 'k');
    switched.new_file_buffer().unwrap();
    switched.buffer = Box::new(crate::buffer::PieceTable::from_text("right\nrest"));
    send_control(&mut switched, &mut out, 'k');
    assert_eq!(switched.clipboard, "right\n");
}

#[test]
fn typing_replaces_the_selection_as_one_edit() {
    let mut app = app_with("abc");
    let mut out = Vec::new();
    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::SHIFT);
    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::SHIFT);

    send(&mut app, &mut out, KeyCode::Char('X'), KeyModifiers::SHIFT);
    assert_eq!(app.buffer.to_string(), "Xc");
    assert!(app.selection.active().is_none());
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "abc");
}

#[test]
fn bracketed_paste_normalizes_lines_and_undoes_once() {
    let mut app = app_with("ab");
    let mut out = Vec::new();
    app.buffer.set_cursor(Cursor { row: 0, col: 1 });

    handle_external_paste(&mut app, &mut out, "X\r\nY").unwrap();
    assert_eq!(app.buffer.to_string(), "aX\nYb");
    assert_eq!(app.buffer.cursor(), Cursor { row: 1, col: 1 });
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "ab");
}

#[test]
fn ctrl_a_selects_the_active_buffer() {
    let mut app = app_with("one\ntwo");
    let mut out = Vec::new();

    send(
        &mut app,
        &mut out,
        KeyCode::Char('a'),
        KeyModifiers::CONTROL,
    );

    assert_eq!(
        app.selection.active().unwrap().ordered(),
        (Cursor { row: 0, col: 0 }, Cursor { row: 1, col: 3 })
    );
    assert_eq!(app.buffer.cursor(), Cursor { row: 1, col: 3 });
    assert_eq!(app.clipboard, "one\ntwo");
    assert!(String::from_utf8_lossy(&out).contains("\x1b]52;c;b25lCnR3bw==\x1b\\"));
}

#[test]
fn base64_handles_partial_chunks() {
    assert_eq!(base64(b"a"), "YQ==");
    assert_eq!(base64(b"ab"), "YWI=");
    assert_eq!(base64(b"abc"), "YWJj");
}

#[test]
fn unicode_copy_uses_st_terminated_osc52_and_internal_clipboard() {
    let mut app = app_with("猫🙂");
    let mut out = Vec::new();

    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::SHIFT);
    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::SHIFT);
    out.clear();
    send(
        &mut app,
        &mut out,
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    );

    assert_eq!(app.clipboard, "猫🙂");
    assert!(String::from_utf8(out)
        .unwrap()
        .contains("\x1b]52;c;54yr8J+Zgg==\x1b\\"));
}

#[test]
fn empty_selection_does_not_replace_or_export_the_clipboard() {
    let mut app = app_with("abc");
    app.clipboard = "keep".to_string();
    let mut out = Vec::new();

    send(
        &mut app,
        &mut out,
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    );

    assert_eq!(app.clipboard, "keep");
    assert!(!String::from_utf8_lossy(&out).contains("\x1b]52;"));
}

#[test]
fn oversized_selection_remains_internal_without_osc52() {
    let text = "a".repeat(OSC52_MAX_BYTES + 1);
    let mut app = app_with(&text);
    let mut out = Vec::new();

    send(
        &mut app,
        &mut out,
        KeyCode::Char('a'),
        KeyModifiers::CONTROL,
    );
    out.clear();
    send(
        &mut app,
        &mut out,
        KeyCode::Char('c'),
        KeyModifiers::CONTROL,
    );

    assert_eq!(app.clipboard, text);
    assert!(!String::from_utf8_lossy(&out).contains("\x1b]52;"));
}
