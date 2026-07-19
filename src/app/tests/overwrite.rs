//! Purpose: verify Insert-key overwrite behavior through the App input boundary.
//! Owns: Unicode replacement, boundary, selection, paste, prompt, and undo assertions.
//! Must not: require a real terminal, filesystem, Project service, or network.
//! Invariants: overwrite affects direct typing only and replaces one grapheme transactionally.
//! Phase: post-v0.1 explicit overwrite mode.

use crossterm::event::{KeyCode, KeyModifiers};

use super::super::*;
use super::make_key;
use crate::buffer::{Cursor, PieceTable};

fn app_with(text: &str) -> App {
    let mut app = App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text(text));
    app.message = None;
    app
}

fn send(app: &mut App, out: &mut Vec<u8>, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_key_with(out, make_key(code, modifiers)).unwrap();
}

fn toggle_overwrite(app: &mut App, out: &mut Vec<u8>) {
    send(app, out, KeyCode::Insert, KeyModifiers::NONE);
}

#[test]
fn insert_toggles_status_cursor_shape_and_ascii_replacement() {
    let mut app = app_with("abc");
    let mut out = Vec::new();

    toggle_overwrite(&mut app, &mut out);
    let enabled = String::from_utf8(std::mem::take(&mut out)).unwrap();
    assert!(
        enabled.contains("OVR"),
        "overwrite status missing: {enabled:?}"
    );
    assert!(
        enabled.contains("\x1b[2 q"),
        "overwrite block cursor missing: {enabled:?}"
    );

    send(&mut app, &mut out, KeyCode::Char('X'), KeyModifiers::SHIFT);
    assert_eq!(app.buffer.to_string(), "Xbc");
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 1 });

    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "abc");
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 0 });

    toggle_overwrite(&mut app, &mut out);
    let disabled = String::from_utf8(out).unwrap();
    assert!(
        disabled.contains("INS"),
        "insert status missing: {disabled:?}"
    );
    assert!(
        disabled.contains("\x1b[0 q"),
        "default cursor restore missing: {disabled:?}"
    );
}

#[test]
fn direct_typing_replaces_one_unicode_grapheme() {
    for source in ["a\u{301}b", "猫b", "🙂b", "👨‍👩‍👧‍👦b"] {
        let mut app = app_with(source);
        let mut out = Vec::new();
        toggle_overwrite(&mut app, &mut out);

        send(&mut app, &mut out, KeyCode::Char('X'), KeyModifiers::SHIFT);

        assert_eq!(app.buffer.to_string(), "Xb", "source {source:?}");
        assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 1 });
        app.buffer.undo();
        assert_eq!(app.buffer.to_string(), source);
    }
}

#[test]
fn overwrite_snaps_an_interior_scalar_cursor_to_the_grapheme_boundary() {
    let mut app = app_with("a\u{301}b");
    app.buffer.set_cursor(Cursor { row: 0, col: 1 });
    let mut out = Vec::new();
    toggle_overwrite(&mut app, &mut out);

    send(&mut app, &mut out, KeyCode::Char('X'), KeyModifiers::SHIFT);

    assert_eq!(app.buffer.to_string(), "Xb");
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "a\u{301}b");
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 0 });
}

#[test]
fn multi_scalar_typed_graphemes_do_not_consume_following_text() {
    for typed in ["e\u{301}", "👩‍💻"] {
        let mut app = app_with("ab");
        let mut out = Vec::new();
        toggle_overwrite(&mut app, &mut out);

        for ch in typed.chars() {
            send(&mut app, &mut out, KeyCode::Char(ch), KeyModifiers::NONE);
        }

        assert_eq!(app.buffer.to_string(), format!("{typed}b"));
    }
}

#[test]
fn end_of_line_newline_end_of_file_and_empty_buffer_insert() {
    let cases = [
        ("ab\ncd", Cursor { row: 0, col: 2 }, "abX\ncd"),
        ("ab", Cursor { row: 0, col: 2 }, "abX"),
        ("\nnext", Cursor { row: 0, col: 0 }, "X\nnext"),
        ("", Cursor { row: 0, col: 0 }, "X"),
    ];
    for (source, cursor, expected) in cases {
        let mut app = app_with(source);
        app.buffer.set_cursor(cursor);
        let mut out = Vec::new();
        toggle_overwrite(&mut app, &mut out);

        send(&mut app, &mut out, KeyCode::Char('X'), KeyModifiers::SHIFT);

        assert_eq!(app.buffer.to_string(), expected, "source {source:?}");
        app.buffer.undo();
        assert_eq!(app.buffer.to_string(), source);
    }
}

#[test]
fn selection_replacement_wins_over_overwrite_mode() {
    let mut app = app_with("abc");
    let mut out = Vec::new();
    toggle_overwrite(&mut app, &mut out);
    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::SHIFT);
    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::SHIFT);

    send(&mut app, &mut out, KeyCode::Char('X'), KeyModifiers::SHIFT);

    assert_eq!(app.buffer.to_string(), "Xc");
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "abc");
}

#[test]
fn bracketed_paste_still_inserts_one_transaction() {
    let mut app = app_with("abc");
    let mut out = Vec::new();
    toggle_overwrite(&mut app, &mut out);

    super::super::input::handle_paste(&mut app, &mut out, "XY").unwrap();

    assert_eq!(app.buffer.to_string(), "XYabc");
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "abc");
}

#[test]
fn overwrite_mode_is_shared_across_buffers_for_the_session() {
    let mut app = app_with("abc");
    let mut out = Vec::new();
    toggle_overwrite(&mut app, &mut out);
    send(
        &mut app,
        &mut out,
        KeyCode::Char('n'),
        KeyModifiers::CONTROL,
    );
    send(&mut app, &mut out, KeyCode::PageUp, KeyModifiers::ALT);

    send(&mut app, &mut out, KeyCode::Char('X'), KeyModifiers::SHIFT);

    assert_eq!(app.buffer.to_string(), "Xbc");
}

#[test]
fn prompt_and_read_only_help_restore_cursor_without_toggling_mode() {
    let mut app = app_with("abc");
    let mut out = Vec::new();
    toggle_overwrite(&mut app, &mut out);

    send(&mut app, &mut out, KeyCode::F(2), KeyModifiers::NONE);
    let prompt = String::from_utf8(std::mem::take(&mut out)).unwrap();
    assert!(prompt.contains("\x1b[0 q"));
    send(&mut app, &mut out, KeyCode::Insert, KeyModifiers::NONE);
    send(&mut app, &mut out, KeyCode::Esc, KeyModifiers::NONE);
    let prompt_closed = String::from_utf8(std::mem::take(&mut out)).unwrap();
    assert!(prompt_closed.contains("\x1b[2 q"));

    send(&mut app, &mut out, KeyCode::F(1), KeyModifiers::NONE);
    let help = String::from_utf8(std::mem::take(&mut out)).unwrap();
    assert!(help.contains("\x1b[0 q"));
    send(&mut app, &mut out, KeyCode::Insert, KeyModifiers::NONE);
    send(&mut app, &mut out, KeyCode::Esc, KeyModifiers::NONE);

    send(&mut app, &mut out, KeyCode::Char('X'), KeyModifiers::SHIFT);
    assert_eq!(app.buffer.to_string(), "Xbc");
}
