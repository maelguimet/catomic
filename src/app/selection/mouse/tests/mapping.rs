//! Purpose: verify mouse mapping across viewport, gutter, and Unicode display boundaries.
//! Owns: focused click fixtures for wrapping, scrolling, empty lines, and terminal cells.
//! Must not: depend on wall-clock timing, a real terminal, filesystem state, or text mutation.
//! Invariants: clicks use zero-based crossterm coordinates and end on grapheme boundaries.

use super::*;

fn click(app: &mut super::super::super::super::App, column: u16, row: u16) {
    let mut out = Vec::new();
    handle_mouse(
        app,
        &mut out,
        event(MouseEventKind::Down(MouseButton::Left), column, row),
    )
    .unwrap();
    handle_mouse(
        app,
        &mut out,
        event(MouseEventKind::Up(MouseButton::Left), column, row),
    )
    .unwrap();
}

#[test]
fn click_maps_tabs_combining_text_wide_text_and_emoji_to_boundaries() {
    let text = "\ta\u{301}猫🙂z";
    for (cell, expected_col) in [
        (0, 0),
        (3, 0),
        (4, 1),
        (5, 3),
        (6, 3),
        (7, 4),
        (8, 4),
        (9, 5),
        (10, 6),
        (40, 6),
    ] {
        let mut app = app_with(text);
        click(&mut app, cell, 0);
        assert_eq!(
            app.buffer.cursor(),
            Cursor {
                row: 0,
                col: expected_col
            }
        );
    }
}

#[test]
fn click_maps_empty_lines_and_clamps_past_end_of_line() {
    let mut empty = app_with("abc\n\nz");
    click(&mut empty, 20, 1);
    assert_eq!(empty.buffer.cursor(), Cursor { row: 1, col: 0 });

    let mut short = app_with("abc\n\nz");
    click(&mut short, 20, 2);
    assert_eq!(short.buffer.cursor(), Cursor { row: 2, col: 1 });
}

#[test]
fn click_maps_unicode_after_horizontal_scroll() {
    let mut app = app_with("aa猫🙂z");
    app.screen.width = 4;
    app.screen.height = 4;
    app.screen.scroll_left = 2;

    click(&mut app, 2, 0);

    assert_eq!(app.screen.scroll_left, 2);
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 3 });
}

#[test]
fn click_maps_unicode_soft_wrap_with_line_number_gutter() {
    let mut app = app_with("a\u{301}猫🙂z");
    app.view_preferences.set_line_numbers(true);
    app.view.soft_wrap = true;
    app.screen.width = 6;
    app.screen.height = 4;

    click(&mut app, 1, 1);
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 3 });

    click(&mut app, 4, 1);
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 4 });
}

#[test]
fn click_maps_first_and_last_visible_content_rows() {
    let text = (0..30)
        .map(|row| format!("line {row}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut app = app_with(&text);
    app.screen.height = 4;
    app.screen.scroll_top = 5;

    click(&mut app, 0, 0);
    assert_eq!(app.buffer.cursor(), Cursor { row: 5, col: 0 });

    click(&mut app, 0, 1);
    assert_eq!(app.buffer.cursor(), Cursor { row: 6, col: 0 });

    click(&mut app, 0, 2);
    assert_eq!(app.buffer.cursor(), Cursor { row: 6, col: 0 });
}

#[test]
fn gutter_click_positions_at_document_line_start() {
    let mut app = app_with("alpha\nbeta");
    app.view_preferences.set_line_numbers(true);

    click(&mut app, 1, 1);

    assert_eq!(app.buffer.cursor(), Cursor { row: 1, col: 0 });
}

#[test]
fn ordinary_click_clears_an_existing_selection() {
    let mut app = app_with("alpha\nbeta");
    app.selection.range = Some(Selection::new(
        Cursor { row: 0, col: 1 },
        Cursor { row: 1, col: 2 },
    ));

    click(&mut app, 3, 0);

    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 3 });
    assert!(app.selection.active().is_none());
}

#[test]
fn complete_status_row_click_preserves_cursor_but_moves_selection_to_chrome() {
    let mut app = app_with("zero\none");
    app.buffer.set_cursor(Cursor { row: 1, col: 2 });
    app.selection.range = Some(Selection::new(
        Cursor { row: 0, col: 1 },
        Cursor { row: 1, col: 2 },
    ));

    click(&mut app, 5, 23);

    assert_eq!(app.buffer.cursor(), Cursor { row: 1, col: 2 });
    assert!(app.selection.active().is_none());
    assert!(app.selection.status.is_none());
}
