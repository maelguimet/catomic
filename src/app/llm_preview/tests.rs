//! Purpose: this file must prove explicit LLM patch preview, confirmation, and undo.
//! Owns: local deterministic App tests for apply, cancel, and stale-source refusal.
//! Must not: construct a network client, contact an endpoint, or access a live model.
//! Invariants: no patch mutates text before Enter; confirmed apply is one undo step.
//! Phase: 6 (LLM, Powerful but Caged).

use super::*;

fn patch() -> &'static str {
    "--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n"
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn preview_is_read_only_until_enter_then_applies_as_one_undo_step() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("one\ntwo\n"));
    let original_position = app.buffer.edit_history_position();
    let mut out = Vec::new();

    show(&mut app, &mut out, patch()).unwrap();
    assert!(is_viewing(&app));
    assert_eq!(app.buffer.to_string(), "one\ntwo\n");
    assert_eq!(app.buffer.edit_history_position(), original_position);
    assert!(String::from_utf8_lossy(&out).contains("-two"));

    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    assert!(!is_viewing(&app));
    assert_eq!(app.buffer.to_string(), "one\nTWO\n");
    assert!(app.file.dirty);
    assert!(app.message.as_deref().unwrap().contains("Ctrl+Z"));

    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "one\ntwo\n");
    assert_eq!(app.buffer.edit_history_position(), original_position);
}

#[test]
fn escape_cancels_without_text_or_history_changes() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("one\ntwo\n"));
    let original_position = app.buffer.edit_history_position();
    let mut out = Vec::new();

    show(&mut app, &mut out, patch()).unwrap();
    handle_key(&mut app, &mut out, key(KeyCode::Esc)).unwrap();

    assert!(!is_viewing(&app));
    assert_eq!(app.buffer.to_string(), "one\ntwo\n");
    assert_eq!(app.buffer.edit_history_position(), original_position);
}

#[test]
fn confirmation_refuses_source_drift_after_preview() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("one\ntwo\n"));
    let mut out = Vec::new();

    show(&mut app, &mut out, patch()).unwrap();
    app.buffer.set_cursor(Cursor { row: 0, col: 0 });
    app.buffer.insert_char('X');
    let drifted = app.buffer.to_string();
    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();

    assert!(!is_viewing(&app));
    assert_eq!(app.buffer.to_string(), drifted);
    assert!(app.message.as_deref().unwrap().contains("not applied"));
}

#[test]
fn plain_start_has_no_preview_or_network_component() {
    let app = super::super::App::new(None).unwrap();
    assert!(app.llm_preview.is_none());
    assert!(!app.caps.network_llm);
    assert!(!app.caps.repo_llm);
}

#[test]
fn marked_region_replacement_previews_then_applies_as_one_undo_step() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("one\ntwo\nthree\n"));
    let original_position = app.buffer.edit_history_position();
    let target = RegionTarget::new(
        Cursor { row: 1, col: 0 },
        Cursor { row: 1, col: 3 },
        "two".to_string(),
    );
    let mut out = Vec::new();

    show_with_region_fallback(
        &mut app,
        &mut out,
        r#"{"catomic_replacement":"TWO"}"#,
        Some(target),
    )
    .unwrap();

    assert!(is_viewing(&app));
    assert_eq!(app.buffer.to_string(), "one\ntwo\nthree\n");
    assert!(String::from_utf8_lossy(&out).contains("+TWO"));
    handle_key(&mut app, &mut out, key(KeyCode::Enter)).unwrap();
    assert_eq!(app.buffer.to_string(), "one\nTWO\nthree\n");
    app.buffer.undo();
    assert_eq!(app.buffer.to_string(), "one\ntwo\nthree\n");
    assert_eq!(app.buffer.edit_history_position(), original_position);
}

#[test]
fn arbitrary_prose_is_not_treated_as_a_marked_region_replacement() {
    let mut app = super::super::App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("one"));
    let target = RegionTarget::new(
        Cursor { row: 0, col: 0 },
        Cursor { row: 0, col: 3 },
        "one".to_string(),
    );
    let mut out = Vec::new();

    show_with_region_fallback(&mut app, &mut out, "Replace it with TWO.", Some(target)).unwrap();

    assert!(!is_viewing(&app));
    assert_eq!(app.buffer.to_string(), "one");
    assert!(app.message.as_deref().unwrap().contains("Invalid"));
}
