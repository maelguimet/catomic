//! Purpose: verify terminal-to-document mouse mapping and selection gestures.
//! Owns: synthetic click, drag, status-row, and double-click fixtures.
//! Must not: require a real terminal, timing sleeps, filesystem, or text mutation.
//! Invariants: all assertions use crossterm's zero-based mouse coordinates.
//! Phase: 3-e mouse selection interaction.

use super::*;
use crossterm::event::KeyModifiers;

fn event(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

fn app_with(text: &str) -> super::super::super::App {
    let mut app = super::super::super::App::new(None).unwrap();
    app.buffer = Box::new(crate::buffer::PieceTable::from_text(text));
    app
}

#[test]
fn click_maps_through_both_viewport_offsets() {
    let mut app = app_with("zero\nabcdef\nuvwxyz");
    app.screen.scroll_top = 1;
    app.screen.scroll_left = 2;
    let mut out = Vec::new();

    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Down(MouseButton::Left), 1, 1),
    )
    .unwrap();

    assert_eq!(app.buffer.cursor(), Cursor { row: 2, col: 3 });
    assert!(app.selection.active().is_none());
}

#[test]
fn click_subtracts_the_line_number_gutter() {
    let mut app = app_with("abcdef");
    app.view.line_numbers = true;
    let mut out = Vec::new();

    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Down(MouseButton::Left), 5, 0),
    )
    .unwrap();

    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 3 });
}

#[test]
fn click_maps_a_wrapped_continuation_to_its_document_column() {
    let mut app = app_with("abcdef");
    app.view.soft_wrap = true;
    app.screen.width = 3;
    app.screen.height = 4;
    let mut out = Vec::new();

    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Down(MouseButton::Left), 1, 1),
    )
    .unwrap();

    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 4 });
}

#[test]
fn left_drag_creates_a_multiline_half_open_selection() {
    let mut app = app_with("zero\nmiddle\nlast");
    let mut out = Vec::new();
    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Down(MouseButton::Left), 1, 0),
    )
    .unwrap();
    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Drag(MouseButton::Left), 3, 1),
    )
    .unwrap();
    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Up(MouseButton::Left), 3, 1),
    )
    .unwrap();

    assert_eq!(
        app.selection.active().unwrap().ordered(),
        (Cursor { row: 0, col: 1 }, Cursor { row: 1, col: 3 })
    );
    assert_eq!(app.buffer.cursor(), Cursor { row: 1, col: 3 });
}

#[test]
fn second_click_on_a_word_expands_to_word_bounds() {
    let mut app = app_with("zero alpha!");
    let mut out = Vec::new();
    let down = event(MouseEventKind::Down(MouseButton::Left), 6, 0);
    let up = event(MouseEventKind::Up(MouseButton::Left), 6, 0);

    handle_mouse(&mut app, &mut out, down).unwrap();
    handle_mouse(&mut app, &mut out, up).unwrap();
    handle_mouse(&mut app, &mut out, down).unwrap();

    assert_eq!(
        app.selection.active().unwrap().ordered(),
        (Cursor { row: 0, col: 5 }, Cursor { row: 0, col: 10 })
    );
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 10 });
}

#[test]
fn click_on_status_row_is_ignored() {
    let mut app = app_with("zero");
    let mut out = Vec::new();

    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Down(MouseButton::Left), 2, 23),
    )
    .unwrap();

    assert_eq!(app.buffer.cursor(), Cursor::default());
    assert!(out.is_empty());
}
