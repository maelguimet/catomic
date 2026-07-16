//! Purpose: verify the built-in shortcut reference and its read-only lifecycle.
//! Owns: focused help key, navigation, and source-preservation regression tests.
//! Must not: touch disk, spawn services, access network, or depend on a real terminal.
//! Invariants: opening and closing help never changes the source buffer.
//! Phase: post-v0.1 core usability.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Cursor, PieceTable};

use super::*;

fn app() -> crate::app::App {
    let mut app = crate::app::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("source text"));
    app
}

#[test]
fn ctrl_h_opens_navigates_and_closes_without_editing_source() {
    let mut app = app();
    let mut out = Vec::new();
    let toggle = KeyEvent::new(KeyCode::Char('h'), KeyModifiers::CONTROL);

    assert!(handle_key(&mut app, &mut out, toggle).unwrap());
    assert!(is_viewing(&app));
    assert!(display_buffer(&app)
        .unwrap()
        .to_string()
        .contains("Ctrl+Shift+S"));

    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
    )
    .unwrap();
    assert!(display_buffer(&app).unwrap().cursor().row > 0);

    assert!(handle_key(&mut app, &mut out, toggle).unwrap());
    assert!(!is_viewing(&app));
    assert_eq!(app.buffer.to_string(), "source text");
}

#[test]
fn help_rejects_edits_and_escape_restores_source_viewport() {
    let mut app = app();
    let mut out = Vec::new();
    app.buffer = Box::new(PieceTable::from_text("a\nb\nc\nsource"));
    app.buffer.set_cursor(Cursor { row: 3, col: 0 });
    app.screen.height = 2;
    app.screen.scroll_top = 3;
    show(&mut app, &mut out).unwrap();
    let help_before = display_buffer(&app).unwrap().to_string();

    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
    )
    .unwrap();
    assert_eq!(display_buffer(&app).unwrap().to_string(), help_before);

    handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    )
    .unwrap();
    assert!(!is_viewing(&app));
    assert_eq!(app.screen.scroll_top, 3);
}
