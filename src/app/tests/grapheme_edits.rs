//! Purpose: regress edit-created Unicode grapheme joins through App input.
//! Owns: cursor-boundary, navigation, deletion, and undo/redo coverage.
//! Must not: bypass semantic input paths or require a real terminal.

use crossterm::event::{KeyCode, KeyModifiers};

use super::super::*;
use super::make_key;
use crate::buffer::{Cursor, PieceTable};

fn app_with(text: &str) -> App {
    let mut app = App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text(text));
    app
}

fn send(app: &mut App, out: &mut Vec<u8>, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_key_with(out, make_key(code, modifiers)).unwrap();
}

fn undo(app: &mut App, out: &mut Vec<u8>) {
    send(app, out, KeyCode::Char('z'), KeyModifiers::CONTROL);
}

fn redo(app: &mut App, out: &mut Vec<u8>) {
    send(
        app,
        out,
        KeyCode::Char('z'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT,
    );
}

#[test]
fn delete_that_joins_regional_indicators_snaps_forward_and_undoes_redoes() {
    let mut app = app_with("🇫x🇷!");
    let mut out = Vec::new();

    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::NONE);
    send(&mut app, &mut out, KeyCode::Delete, KeyModifiers::NONE);

    assert_eq!(app.buffer.to_string(), "🇫🇷!");
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 2 });

    send(&mut app, &mut out, KeyCode::Left, KeyModifiers::NONE);
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 0 });
    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::NONE);
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 2 });

    send(&mut app, &mut out, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(app.buffer.to_string(), "!");

    undo(&mut app, &mut out);
    assert_eq!(app.buffer.to_string(), "🇫🇷!");
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 2 });
    undo(&mut app, &mut out);
    assert_eq!(app.buffer.to_string(), "🇫x🇷!");
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 1 });

    redo(&mut app, &mut out);
    assert_eq!(app.buffer.to_string(), "🇫🇷!");
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 2 });
    redo(&mut app, &mut out);
    assert_eq!(app.buffer.to_string(), "!");
}

#[test]
fn typing_joining_zwj_snaps_forward_and_backspace_removes_the_whole_grapheme() {
    let mut app = app_with("👩💻!");
    let mut out = Vec::new();

    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::NONE);
    send(
        &mut app,
        &mut out,
        KeyCode::Char('\u{200d}'),
        KeyModifiers::NONE,
    );

    assert_eq!(app.buffer.to_string(), "👩\u{200d}💻!");
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 3 });

    send(&mut app, &mut out, KeyCode::Backspace, KeyModifiers::NONE);
    assert_eq!(app.buffer.to_string(), "!");
    undo(&mut app, &mut out);
    assert_eq!(app.buffer.to_string(), "👩\u{200d}💻!");
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 3 });
    undo(&mut app, &mut out);
    assert_eq!(app.buffer.to_string(), "👩💻!");
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 1 });
    redo(&mut app, &mut out);
    assert_eq!(app.buffer.to_string(), "👩\u{200d}💻!");
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 3 });
}

#[test]
fn paste_and_range_replacement_snap_forward_after_joining_graphemes() {
    let mut paste = app_with("👩💻!");
    let mut out = Vec::new();
    paste.buffer.set_cursor(Cursor { row: 0, col: 1 });

    super::super::input::handle_paste(&mut paste, &mut out, "\u{200d}").unwrap();
    assert_eq!(paste.buffer.to_string(), "👩\u{200d}💻!");
    assert_eq!(paste.buffer.cursor(), Cursor { row: 0, col: 3 });
    undo(&mut paste, &mut out);
    assert_eq!(paste.buffer.to_string(), "👩💻!");
    redo(&mut paste, &mut out);
    assert_eq!(paste.buffer.cursor(), Cursor { row: 0, col: 3 });

    let mut replacement = app_with("👩x💻!");
    replacement.buffer.set_cursor(Cursor { row: 0, col: 1 });
    send(
        &mut replacement,
        &mut out,
        KeyCode::Right,
        KeyModifiers::SHIFT,
    );
    send(
        &mut replacement,
        &mut out,
        KeyCode::Char('\u{200d}'),
        KeyModifiers::NONE,
    );
    assert_eq!(replacement.buffer.to_string(), "👩\u{200d}💻!");
    assert_eq!(replacement.buffer.cursor(), Cursor { row: 0, col: 3 });
    undo(&mut replacement, &mut out);
    assert_eq!(replacement.buffer.to_string(), "👩x💻!");
    redo(&mut replacement, &mut out);
    assert_eq!(replacement.buffer.cursor(), Cursor { row: 0, col: 3 });
}
