//! Purpose: verify keyboard selection, clipboard, and selection-aware App edits.
//! Owns: captured key/render fixtures and bracketed-paste integration assertions.
//! Must not: require a real terminal, system clipboard reader, mouse, or filesystem.
//! Invariants: selection replacement and paste each undo as one transaction.
//! Phase: 3-d keyboard selection and clipboard interaction.

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

#[test]
fn shift_arrows_select_and_ctrl_c_populates_both_clipboards() {
    let mut app = app_with("abc");
    let mut out = Vec::new();

    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::SHIFT);
    send(&mut app, &mut out, KeyCode::Right, KeyModifiers::SHIFT);
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
    assert!(String::from_utf8(out)
        .unwrap()
        .contains("\x1b]52;c;YWI=\x07"));
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
}

#[test]
fn base64_handles_partial_chunks() {
    assert_eq!(base64(b"a"), "YQ==");
    assert_eq!(base64(b"ab"), "YWI=");
    assert_eq!(base64(b"abc"), "YWJj");
}
