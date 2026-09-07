//! Exercise autoindent at the insertion point through default-key dispatch.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::App;
use crate::buffer::{Cursor, PieceTable};

fn app(text: &str) -> App {
    let mut app = App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text(text));
    app
}

fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.handle_key_with(&mut Vec::new(), KeyEvent::new(code, modifiers))
        .unwrap();
}

fn assert_enter_and_undo(app: &mut App, expected: &str, cursor: Cursor) {
    let original = app.buffer.to_string();
    press(app, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(app.buffer.to_string(), expected);
    assert_eq!(app.buffer.cursor(), cursor);
    press(app, KeyCode::Char('z'), KeyModifiers::CONTROL);
    assert_eq!(app.buffer.to_string(), original);
}

#[test]
fn home_enter_preserves_the_following_lines_indentation() {
    for source in ["    foo", "\tfoo", " \t  foo", "    if ready {"] {
        let mut app = app(source);
        press(&mut app, KeyCode::End, KeyModifiers::NONE);
        press(&mut app, KeyCode::Home, KeyModifiers::NONE);
        assert_enter_and_undo(&mut app, &format!("\n{source}"), Cursor { row: 1, col: 0 });
    }
}

#[test]
fn enter_inside_indentation_copies_only_whitespace_before_the_cursor() {
    for (source, col, expected) in [
        ("    foo", 2, "  \n    foo"),
        ("\t\tfoo", 1, "\t\n\t\tfoo"),
        (" \t  foo", 2, " \t\n \t  foo"),
        ("    if ready {", 1, " \n    if ready {"),
    ] {
        let mut app = app(source);
        for _ in 0..col {
            press(&mut app, KeyCode::Right, KeyModifiers::NONE);
        }
        assert_enter_and_undo(&mut app, expected, Cursor { row: 1, col });
    }
}

#[test]
fn end_enter_keeps_inherited_indentation() {
    for (source, prefix) in [("    foo", "    "), (" \tfoo", " \t"), ("   ", "   ")] {
        let mut app = app(source);
        press(&mut app, KeyCode::End, KeyModifiers::NONE);
        assert_enter_and_undo(
            &mut app,
            &format!("{source}\n{prefix}"),
            Cursor {
                row: 1,
                col: prefix.chars().count(),
            },
        );
    }
}

#[test]
fn end_enter_after_a_block_opener_keeps_the_configured_extra_level() {
    for opener in ['{', '[', '(', ':'] {
        let source = format!("  block {opener}  ");
        let mut app = app(&source);
        app.editor_config =
            crate::config::editor::parse("[editor]\ntab_size = 2\n[languages.rs]\ntab_size = 3\n")
                .unwrap();
        app.file.path = Some("main.rs".into());
        press(&mut app, KeyCode::End, KeyModifiers::NONE);
        assert_enter_and_undo(
            &mut app,
            &format!("{source}\n     "),
            Cursor { row: 1, col: 5 },
        );
    }
}

#[test]
fn enter_before_a_block_opener_does_not_add_an_extra_level() {
    let mut app = app("  if ready {");
    press(&mut app, KeyCode::End, KeyModifiers::NONE);
    press(&mut app, KeyCode::Left, KeyModifiers::NONE);
    assert_enter_and_undo(&mut app, "  if ready \n  {", Cursor { row: 1, col: 2 });
}

#[test]
fn enter_replacing_leading_whitespace_uses_the_selection_start() {
    let mut app = app("    foo");
    for _ in 0..2 {
        press(&mut app, KeyCode::Right, KeyModifiers::SHIFT);
    }
    assert_enter_and_undo(&mut app, "\n  foo", Cursor { row: 1, col: 0 });
}
