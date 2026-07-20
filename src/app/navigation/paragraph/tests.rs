//! Purpose: verify paragraph boundaries, clamping, selection rules, and Unicode columns.
//! Owns: focused Ctrl+Up/Down prose-navigation fixtures.
//! Must not: access disk/network, mutate through navigation, or require a terminal.
//! Invariants: movement never changes bytes or edit history.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Cursor, PieceTable};

use super::super::handle_key;

fn app(text: &str) -> crate::app::App {
    let mut app = crate::app::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text(text));
    app
}

fn control_arrow(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn move_paragraph(app: &mut crate::app::App, code: KeyCode) {
    assert!(handle_key(app, &mut Vec::new(), control_arrow(code)).unwrap());
}

#[test]
fn down_skips_current_paragraph_and_runs_of_blank_lines() {
    let mut app = app("alpha\ncontinued\n\n \t\nnext\ncontinued next\n\nlast");
    app.buffer.set_cursor(Cursor { row: 1, col: 3 });

    move_paragraph(&mut app, KeyCode::Down);
    assert_eq!(app.buffer.cursor(), Cursor { row: 4, col: 3 });
    move_paragraph(&mut app, KeyCode::Down);
    assert_eq!(app.buffer.cursor(), Cursor { row: 7, col: 3 });
    move_paragraph(&mut app, KeyCode::Down);
    assert_eq!(app.buffer.cursor(), Cursor { row: 7, col: 3 });
}

#[test]
fn up_moves_to_current_start_then_previous_paragraph_start() {
    let mut app = app("first\ncontinued\n\n\nsecond\ncontinued second");
    app.buffer.set_cursor(Cursor { row: 5, col: 2 });

    move_paragraph(&mut app, KeyCode::Up);
    assert_eq!(app.buffer.cursor(), Cursor { row: 4, col: 2 });
    move_paragraph(&mut app, KeyCode::Up);
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 2 });
    move_paragraph(&mut app, KeyCode::Up);
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 2 });
}

#[test]
fn movement_from_blank_lines_uses_the_adjacent_paragraph() {
    let mut app = app("first\n\n\nsecond");
    app.buffer.set_cursor(Cursor { row: 2, col: 0 });
    move_paragraph(&mut app, KeyCode::Up);
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 0 });

    app.buffer.set_cursor(Cursor { row: 1, col: 0 });
    move_paragraph(&mut app, KeyCode::Down);
    assert_eq!(app.buffer.cursor(), Cursor { row: 3, col: 0 });
}

#[test]
fn empty_crlf_and_trailing_blank_documents_clamp_safely() {
    let mut empty = app("");
    move_paragraph(&mut empty, KeyCode::Up);
    move_paragraph(&mut empty, KeyCode::Down);
    assert_eq!(empty.buffer.cursor(), Cursor::default());

    let mut crlf = app("one\r\ntwo\r\n\r\nnext\r\n\r\n");
    crlf.buffer.set_cursor(Cursor { row: 1, col: 1 });
    move_paragraph(&mut crlf, KeyCode::Down);
    assert_eq!(crlf.buffer.cursor(), Cursor { row: 3, col: 1 });
    move_paragraph(&mut crlf, KeyCode::Down);
    assert_eq!(crlf.buffer.cursor(), Cursor { row: 5, col: 0 });
}

#[test]
fn visual_column_snaps_across_tabs_wide_and_combining_graphemes() {
    let mut wide = app("\tcat\n\n12猫a\u{301}x");
    wide.buffer.set_cursor(Cursor { row: 0, col: 1 });

    move_paragraph(&mut wide, KeyCode::Down);

    assert_eq!(wide.buffer.cursor(), Cursor { row: 2, col: 3 });
    let target = wide.buffer.line(2).unwrap();
    assert_eq!(crate::editor::text_layout::scalar_to_cell(&target, 3), 4);

    let mut combining = app("abc\n\n猫a\u{301}x");
    combining.buffer.set_cursor(Cursor { row: 0, col: 3 });
    move_paragraph(&mut combining, KeyCode::Down);
    assert_eq!(combining.buffer.cursor(), Cursor { row: 2, col: 3 });
    assert_eq!(
        crate::editor::text_layout::snap_to_grapheme_col("猫a\u{301}x", 3),
        combining.buffer.cursor().col
    );
}

#[test]
fn control_shift_down_does_not_infer_paragraph_selection_extension() {
    let mut app = app("first\ncontinued\n\nnext");
    let mut out = Vec::new();
    crate::app::selection::handle_shortcut(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
    )
    .unwrap();
    app.buffer.set_cursor(Cursor::default());

    crate::app::input::handle_key_with(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL | KeyModifiers::SHIFT),
    )
    .unwrap();

    assert_eq!(app.buffer.cursor(), Cursor { row: 1, col: 0 });
    assert!(app.selection.active().is_none());
}

#[test]
fn plain_control_movement_clears_selection_without_editing() {
    let mut app = app("first\ncontinued\n\nnext");
    let original = app.buffer.to_string();
    let history = app.buffer.edit_history_position();
    let mut out = Vec::new();
    crate::app::selection::handle_shortcut(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert!(app.selection.active().is_some());

    move_paragraph(&mut app, KeyCode::Up);

    assert!(app.selection.active().is_none());
    assert_eq!(app.buffer.to_string(), original);
    assert_eq!(app.buffer.edit_history_position(), history);
    assert!(!app.file.dirty);
}
