//! Purpose: verify mouse clicks never reposition a source hidden by a read-only view.
//! Owns: representative view-open, click, close, and resumed-source interaction fixtures.
//! Must not: contact a model, start a process, write files, or inspect private view buffers.
//! Invariants: hidden source text and cursor remain stable until the view is explicitly closed.
//! Phase: beta mouse click regression coverage.

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn click(app: &mut super::super::super::super::App, column: u16, row: u16) {
    let mut out = Vec::new();
    for kind in [
        MouseEventKind::Down(MouseButton::Left),
        MouseEventKind::Up(MouseButton::Left),
    ] {
        handle_mouse(app, &mut out, event(kind, column, row)).unwrap();
    }
}

#[test]
fn llm_answer_click_cannot_reposition_hidden_source() {
    let mut app = app_with("source\nline");
    let source_cursor = Cursor { row: 1, col: 2 };
    app.buffer.set_cursor(source_cursor);
    let source_text = app.buffer.to_string();
    let mut out = Vec::new();
    super::super::super::super::llm_answer::show(&mut app, &mut out, "answer\nview").unwrap();
    let display_cursor = super::super::super::super::view::display_buffer(&app).cursor();

    click(&mut app, 0, 0);

    assert_eq!(app.buffer.cursor(), source_cursor);
    assert_eq!(app.buffer.to_string(), source_text);
    assert_eq!(
        super::super::super::super::view::display_buffer(&app).cursor(),
        display_cursor
    );
}

#[test]
fn click_positions_source_after_read_only_view_closes() {
    let mut app = app_with("source\nline");
    let mut out = Vec::new();
    super::super::super::super::llm_answer::show(&mut app, &mut out, "answer").unwrap();
    super::super::super::super::llm_answer::handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    )
    .unwrap();

    click(&mut app, 3, 1);

    assert_eq!(app.buffer.cursor(), Cursor { row: 1, col: 3 });
}

#[test]
fn markdown_preview_click_is_ignored_and_click_after_f6_exit_positions_source() {
    let mut app = app_with("# heading\nbody");
    app.file.path = Some("note.md".into());
    app.buffer.set_cursor(Cursor { row: 1, col: 2 });
    let mut out = Vec::new();
    let f6 = KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE);
    super::super::super::super::view::handle_key(&mut app, &mut out, f6).unwrap();

    click(&mut app, 1, 0);
    assert_eq!(app.buffer.cursor(), Cursor { row: 1, col: 2 });

    super::super::super::super::view::handle_key(&mut app, &mut out, f6).unwrap();
    click(&mut app, 1, 0);
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 1 });
}

#[test]
fn command_prompt_blocks_click_and_click_after_escape_positions_source() {
    let mut app = app_with("alpha\nbeta");
    app.buffer.set_cursor(Cursor { row: 1, col: 2 });
    let mut out = Vec::new();
    super::super::super::super::command_prompt::open_command_prompt(&mut app, &mut out).unwrap();

    click(&mut app, 1, 0);
    assert_eq!(app.buffer.cursor(), Cursor { row: 1, col: 2 });

    super::super::super::super::command_prompt::handle_active_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    )
    .unwrap();
    click(&mut app, 1, 0);
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 1 });
}
