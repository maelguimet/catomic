//! Purpose: verify terminal-to-document mouse mapping and selection gestures.
//! Owns: synthetic click, drag, status-row, and double-click fixtures.
//! Must not: require a real terminal, timing sleeps, filesystem, or text mutation.
//! Invariants: all assertions use crossterm's zero-based mouse coordinates.

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

mod mapping;
mod views;

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
fn click_cancels_a_pending_confirmation_and_message() {
    let mut app = app_with("abc");
    app.pending_quit_confirm = true;
    app.message = Some("Press Ctrl+Q again to quit without saving.".to_string());
    let mut out = Vec::new();

    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Down(MouseButton::Left), 1, 0),
    )
    .unwrap();

    assert!(!app.pending_quit_confirm);
    assert!(app.message.is_none());
}

#[test]
fn click_subtracts_the_line_number_gutter() {
    let mut app = app_with("abcdef");
    app.view_preferences.set_line_numbers(true);
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
    assert_eq!(app.clipboard, "ero\nmid");
    assert!(String::from_utf8_lossy(&out).contains("\x1b]52;c;ZXJvCm1pZA==\x1b\\"));
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
fn status_row_mouse_down_starts_chrome_selection_without_moving_cursor() {
    let mut app = app_with("zero");
    let mut out = Vec::new();

    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Down(MouseButton::Left), 2, 23),
    )
    .unwrap();

    assert_eq!(app.buffer.cursor(), Cursor::default());
    assert!(app.selection.is_status_dragging());
    assert!(!out.is_empty());
}

#[test]
fn status_path_drag_copies_on_select_and_ctrl_c_uses_the_same_path() {
    let mut app = app_with("zero");
    app.file.path = Some("/work/cats/notes.txt".into());
    let mut out = Vec::new();

    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Down(MouseButton::Left), 8, 23),
    )
    .unwrap();
    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Drag(MouseButton::Left), 28, 23),
    )
    .unwrap();
    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Up(MouseButton::Left), 28, 23),
    )
    .unwrap();

    let status = super::super::super::render::status_line(&app);
    assert_eq!(
        app.selection.status_range(&status.text),
        Some((8, status.text.len()))
    );
    assert_eq!(app.clipboard, "/work/cats/notes.txt");
    assert!(String::from_utf8_lossy(&out).contains("\x1b]52;c;L3dvcmsvY2F0cy9ub3Rlcy50eHQ=\x1b\\"));

    out.clear();
    app.handle_key_with(
        &mut out,
        KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert_eq!(app.clipboard, "/work/cats/notes.txt");
    assert!(String::from_utf8_lossy(&out).contains("\x1b]52;c;L3dvcmsvY2F0cy9ub3Rlcy50eHQ=\x1b\\"));
}

#[test]
fn click_after_wheel_scroll_maps_through_the_new_origin() {
    let text = (0..12)
        .map(|row| format!("line-{row}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut app = app_with(&text);
    app.screen.height = 5;
    let mut out = Vec::new();

    handle_mouse(&mut app, &mut out, event(MouseEventKind::ScrollDown, 0, 0)).unwrap();
    assert_eq!(app.screen.scroll_top, 3);
    assert_eq!(app.buffer.cursor(), Cursor::default());

    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Down(MouseButton::Left), 5, 0),
    )
    .unwrap();
    assert_eq!(app.buffer.cursor(), Cursor { row: 3, col: 5 });
}

#[test]
fn click_after_wrapped_wheel_scroll_maps_wide_content_from_wrap_origin() {
    let mut app = app_with("ab猫cdefghijklmnopqrstuvwxyz");
    app.view.soft_wrap = true;
    app.screen.width = 4;
    app.screen.height = 4;
    let mut out = Vec::new();
    handle_mouse(&mut app, &mut out, event(MouseEventKind::ScrollDown, 0, 0)).unwrap();
    assert!(app.screen.wrap_col > 0);
    let first = crate::terminal::render::wrapped::visible_rows(
        &*app.buffer,
        app.screen.scroll_top,
        app.screen.wrap_col,
        app.screen.visible_height(),
        super::super::super::view::content_width(&app),
    )
    .unwrap()
    .remove(0);
    let expected_col = first.start_col + text_layout::scalar_at_cell(&first.content, 2);

    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Down(MouseButton::Left), 2, 0),
    )
    .unwrap();

    assert_eq!(
        app.buffer.cursor(),
        Cursor {
            row: first.document_row,
            col: expected_col,
        }
    );
}

#[test]
fn configured_mouse_gestures_are_remapped_and_defaults_can_be_unbound() {
    let mut app = app_with("zero alpha!");
    app.keybindings = crate::config::keybindings::parse(
        "[keybindings]\nmouse-place-cursor = []\nmouse-select-word = [\"mouse-left\"]\n",
    )
    .unwrap();
    let mut out = Vec::new();

    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Down(MouseButton::Left), 6, 0),
    )
    .unwrap();

    assert_eq!(
        app.selection.active().unwrap().ordered(),
        (Cursor { row: 0, col: 5 }, Cursor { row: 0, col: 10 })
    );
}

#[test]
fn configured_wheel_gestures_change_direction_and_can_be_unbound() {
    let text = (0..20)
        .map(|row| format!("line-{row}"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut app = app_with(&text);
    app.screen.height = 5;
    app.screen.scroll_top = 6;
    app.keybindings = crate::config::keybindings::parse(
        "[keybindings]\nmouse-scroll-up = []\nmouse-scroll-down = [\"mouse-wheel-up\"]\n",
    )
    .unwrap();
    let mut out = Vec::new();

    handle_mouse(&mut app, &mut out, event(MouseEventKind::ScrollUp, 0, 0)).unwrap();
    assert_eq!(app.screen.scroll_top, 9, "remapped wheel moved down");

    out.clear();
    handle_mouse(&mut app, &mut out, event(MouseEventKind::ScrollDown, 0, 0)).unwrap();
    assert_eq!(app.screen.scroll_top, 9, "unbound gesture did nothing");
    assert!(out.is_empty());
}

#[test]
fn mobile_two_tap_selection_ends_on_a_wrapped_unicode_grapheme_boundary() {
    let mut app = app_with("a\u{301}\t猫🙂z");
    super::super::super::mobile::configure(&mut app, true);
    app.view_preferences.set_line_numbers(true);
    app.view.soft_wrap = true;
    app.screen.update_size(8, 6);
    super::super::begin_touch_selection(&mut app);
    let mut out = Vec::new();

    handle_mouse(
        &mut app,
        &mut out,
        event(MouseEventKind::Down(MouseButton::Left), 4, 1),
    )
    .unwrap();

    let (_, end) = app.selection.active().unwrap().ordered();
    assert_eq!(end, app.buffer.cursor());
    let (start, end) = app.selection.active().unwrap().ordered();
    assert_eq!(app.clipboard, app.buffer.text_range(start, end).unwrap());
    assert!(String::from_utf8_lossy(&out).contains("\x1b]52;c;"));
    assert_eq!(
        crate::editor::text_layout::ceil_to_grapheme_col(&app.buffer.line(0).unwrap(), end.col),
        end.col,
    );
    assert!(!super::super::is_touch_selecting(&app));
}
