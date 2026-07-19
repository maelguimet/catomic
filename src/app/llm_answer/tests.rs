//! Purpose: this file must prove explanations remain read-only and transient.
//! Owns: answer rendering, non-apply keys, Escape restoration, and Plain startup state.
//! Must not: construct a client, contact an endpoint, or mutate files.
//! Invariants: answer interaction never changes source text or edit history.
//! Phase: 6 (LLM, Powerful but Caged).

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Cursor, PieceTable};

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn explanation_is_read_only_and_escape_restores_source_viewport() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("source text"));
    app.buffer.set_cursor(Cursor { row: 0, col: 8 });
    app.screen.width = 4;
    app.reveal_cursor();
    let source_scroll_left = app.screen.scroll_left;
    let history = app.buffer.edit_history_position();
    let mut out = Vec::new();

    show(&mut app, &mut out, "This code returns the input.").unwrap();
    assert!(is_viewing(&app));
    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    assert_eq!(app.buffer.to_string(), "source text");
    assert_eq!(app.buffer.edit_history_position(), history);

    handle_key(&mut app, &mut out, key(KeyCode::Esc)).unwrap();
    assert!(!is_viewing(&app));
    assert_eq!(app.screen.scroll_left, source_scroll_left);
    assert_eq!(app.buffer.to_string(), "source text");
}

#[test]
fn plain_start_constructs_no_answer_view() {
    let app = super::super::App::new(None).unwrap();
    assert!(app.surfaces.llm_answer.is_none());
}
