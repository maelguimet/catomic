//! Purpose: verify local completion through the App input seam.
//! Owns: preview, cycling, acceptance, dismissal, and gating tests.
//! Must not: launch a terminal, scan a project, spawn work/processes, or touch disk.
//! Invariants: preview/dismiss do not edit; acceptance is one undoable replacement.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Cursor, PieceTable};

use super::super::App;

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn completion_app() -> App {
    let mut app = App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("alpha alpine alphabet\nal"));
    app.buffer.set_cursor(Cursor { row: 1, col: 2 });
    app
}

#[test]
fn ctrl_space_previews_then_enter_accepts_one_undoable_edit() {
    let mut app = completion_app();
    let original = app.buffer.to_string();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, key(KeyCode::Char(' '), KeyModifiers::CONTROL))
        .unwrap();
    assert_eq!(app.buffer.to_string(), original);
    assert!(app.message.as_deref().unwrap_or("").contains("alpha"));

    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();
    assert_eq!(app.buffer.to_string(), "alpha alpine alphabet\nalpha");
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), original);
}

#[test]
fn tab_triggers_and_cycles_while_escape_dismisses_without_editing() {
    let mut app = completion_app();
    let original = app.buffer.to_string();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, key(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    assert!(app.message.as_deref().unwrap_or("").contains("alpha"));
    app.handle_key_with(&mut out, key(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();
    assert!(app.message.as_deref().unwrap_or("").contains("alphabet"));
    app.handle_key_with(&mut out, key(KeyCode::Esc, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.to_string(), original);
    assert!(app.message.is_none());
}

#[test]
fn tab_without_completion_uses_language_tab_stop_as_one_edit() {
    let mut app = App::new(None).unwrap();
    app.editor_config =
        crate::config::editor::parse("[editor]\ntab_size = 3\n[languages.rs]\ntab_size = 4\n")
            .unwrap();
    app.file.path = Some("main.rs".into());
    app.buffer = Box::new(PieceTable::from_text("  "));
    app.buffer.set_cursor(Cursor { row: 0, col: 2 });
    let mut out = Vec::new();

    app.handle_key_with(&mut out, key(KeyCode::Tab, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "    ");
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "  ");
}

#[test]
fn changed_prefix_is_refused_instead_of_replaced() {
    let mut app = completion_app();
    let mut out = Vec::new();
    app.handle_key_with(&mut out, key(KeyCode::Char(' '), KeyModifiers::CONTROL))
        .unwrap();
    app.buffer.insert_char('x');

    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "alpha alpine alphabet\nalx");
    assert!(app
        .message
        .as_deref()
        .unwrap_or("")
        .contains("prefix changed"));
}

#[test]
fn ordinary_typing_dismisses_completion_and_falls_through() {
    let mut app = completion_app();
    let mut out = Vec::new();
    app.handle_key_with(&mut out, key(KeyCode::Char(' '), KeyModifiers::CONTROL))
        .unwrap();

    app.handle_key_with(&mut out, key(KeyCode::Char('x'), KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "alpha alpine alphabet\nalx");
    assert!(app.message.is_none());
}

#[test]
fn prefix_beyond_bound_is_refused_without_partial_replacement() {
    let mut app = App::new(None).unwrap();
    let prefix = "a".repeat(super::PREFIX_COLS + 1);
    app.buffer = Box::new(PieceTable::from_text(&format!("{prefix}long\n{prefix}")));
    app.buffer.set_cursor(Cursor {
        row: 1,
        col: prefix.len(),
    });
    let original = app.buffer.to_string();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, key(KeyCode::Char(' '), KeyModifiers::CONTROL))
        .unwrap();

    assert_eq!(app.buffer.to_string(), original);
    assert!(app.message.as_deref().unwrap_or("").contains("exceeds"));
}

#[test]
fn null_form_of_ctrl_space_opens_and_cycles() {
    let mut app = completion_app();
    let mut out = Vec::new();

    app.handle_key_with(&mut out, key(KeyCode::Null, KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.message.as_deref().unwrap_or("").contains("alpha"));
    app.handle_key_with(&mut out, key(KeyCode::Null, KeyModifiers::CONTROL))
        .unwrap();
    assert!(app.message.as_deref().unwrap_or("").contains("alphabet"));
}

#[test]
fn path_shaped_text_still_uses_only_local_words() {
    let mut app = App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("main src/ma"));
    app.buffer.set_cursor(Cursor { row: 0, col: 11 });
    let mut out = Vec::new();

    app.handle_key_with(&mut out, key(KeyCode::Null, KeyModifiers::CONTROL))
        .unwrap();
    app.handle_key_with(&mut out, key(KeyCode::Enter, KeyModifiers::NONE))
        .unwrap();

    assert_eq!(app.buffer.to_string(), "main src/main");
}
