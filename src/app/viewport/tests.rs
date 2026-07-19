//! Purpose: verify viewport-only scrolling across editable and read-only displays.
//! Owns: wheel bounds, wrapping, resize decoupling, and logical-state regression fixtures.
//! Must not: require a terminal, write files, start services, or mutate through scroll actions.
//! Invariants: scrolling changes only the viewport origin and rendered caret visibility.
//! Phase: post-v0.1 viewport-only wheel scrolling.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::buffer::{Cursor, PieceTable};

use super::*;

fn app_with_lines(count: usize) -> App {
    let text = (0..count)
        .map(|row| format!("row-{row:02}-abcdefghijklmnopqrstuvwxyz"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut app = App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text(&text));
    app.screen.height = 6;
    app
}

#[test]
fn logical_scroll_is_three_rows_bounded_and_preserves_horizontal_offset() {
    let mut app = app_with_lines(20);
    app.view_preferences.set_line_numbers(true);
    app.screen.scroll_left = 7;
    let cursor = app.buffer.cursor();

    assert!(scroll_viewport(&mut app, ScrollDirection::Down, MOUSE_WHEEL_ROWS).unwrap());
    assert_eq!(app.screen.scroll_top, 3);
    assert_eq!(app.screen.scroll_left, 7);
    assert_eq!(app.buffer.cursor(), cursor);

    for _ in 0..100 {
        scroll_viewport(&mut app, ScrollDirection::Down, MOUSE_WHEEL_ROWS).unwrap();
    }
    assert_eq!(app.screen.scroll_top, 15);
    assert!(!scroll_viewport(&mut app, ScrollDirection::Down, MOUSE_WHEEL_ROWS).unwrap());

    for _ in 0..100 {
        scroll_viewport(&mut app, ScrollDirection::Up, MOUSE_WHEEL_ROWS).unwrap();
    }
    assert_eq!(app.screen.scroll_top, 0);
}

#[test]
fn wheel_preserves_selection_bytes_dirty_save_point_and_history() {
    let mut app = app_with_lines(40);
    let mut out = Vec::new();
    crate::app::selection::handle_shortcut(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
    )
    .unwrap();
    let selection = app.selection.active().unwrap();
    let cursor = app.buffer.cursor();
    let text = app.buffer.to_string();
    let history = app.buffer.edit_history_position();
    let save_point = app.file.saved_history_position;

    handle_mouse_wheel(&mut app, &mut out, ScrollDirection::Up, 0).unwrap();

    assert_eq!(app.selection.active(), Some(selection));
    assert_eq!(app.buffer.cursor(), cursor);
    assert_eq!(app.buffer.to_string(), text);
    assert_eq!(app.buffer.edit_history_position(), history);
    assert_eq!(app.file.saved_history_position, save_point);
    assert!(!app.file.dirty);
    assert!(String::from_utf8_lossy(&out).ends_with("\x1b[?25l\x1b[1;1H"));
}

#[test]
fn typing_after_scroll_reveals_and_edits_at_the_original_cursor() {
    let mut app = app_with_lines(30);
    let mut out = Vec::new();
    app.buffer.set_cursor(Cursor { row: 0, col: 4 });

    handle_mouse_wheel(&mut app, &mut out, ScrollDirection::Down, 0).unwrap();
    assert_eq!(app.screen.scroll_top, 3);
    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 4 });

    crate::app::input::handle_key_with(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Char('X'), KeyModifiers::NONE),
    )
    .unwrap();

    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 5 });
    assert!(app.buffer.line(0).unwrap().starts_with("row-X00"));
    assert_eq!(app.screen.scroll_top, 0);
    assert!(app.file.dirty);
}

#[test]
fn keyboard_navigation_after_scroll_moves_from_and_reveals_the_original_cursor() {
    let mut app = app_with_lines(30);
    let mut out = Vec::new();
    app.buffer.set_cursor(Cursor { row: 0, col: 4 });
    handle_mouse_wheel(&mut app, &mut out, ScrollDirection::Down, 0).unwrap();

    crate::app::input::handle_key_with(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
    )
    .unwrap();

    assert_eq!(app.buffer.cursor(), Cursor { row: 1, col: 4 });
    assert_eq!(app.screen.scroll_top, 1);
    assert!(String::from_utf8_lossy(&out).ends_with("\x1b[1;5H\x1b[?25h"));
}

#[test]
fn wrapped_scroll_advances_by_visible_rows_across_tabs_and_unicode() {
    let mut app = App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text(
        "a\t猫b🙂cdefgh\na\u{301}bcdefghijkl\nlast-line",
    ));
    app.view.soft_wrap = true;
    app.view_preferences.set_line_numbers(true);
    app.screen.width = 6;
    app.screen.height = 4;
    let expected = crate::terminal::render::wrapped::visible_rows(
        &*app.buffer,
        0,
        0,
        app.screen.visible_height() + MOUSE_WHEEL_ROWS,
        crate::app::view::content_width(&app),
    )
    .unwrap()[MOUSE_WHEEL_ROWS]
        .clone();

    scroll_viewport(&mut app, ScrollDirection::Down, MOUSE_WHEEL_ROWS).unwrap();

    assert_eq!(
        (app.screen.scroll_top, app.screen.wrap_col),
        (expected.document_row, expected.start_col)
    );
    assert_eq!(app.buffer.cursor(), Cursor::default());
    assert_eq!(app.screen.scroll_left, 0);

    scroll_viewport(&mut app, ScrollDirection::Up, MOUSE_WHEEL_ROWS).unwrap();
    assert_eq!((app.screen.scroll_top, app.screen.wrap_col), (0, 0));
}

#[test]
fn wrapped_scroll_clamps_to_a_full_final_viewport_without_duplicate_rows() {
    let mut app = App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("abcdefghijklmn"));
    app.view.soft_wrap = true;
    app.screen.width = 3;
    app.screen.height = 4;

    for _ in 0..20 {
        scroll_viewport(&mut app, ScrollDirection::Down, MOUSE_WHEEL_ROWS).unwrap();
    }
    let rows = crate::terminal::render::wrapped::visible_rows(
        &*app.buffer,
        app.screen.scroll_top,
        app.screen.wrap_col,
        app.screen.visible_height(),
        crate::app::view::content_width(&app),
    )
    .unwrap();

    assert_eq!(rows.len(), app.screen.visible_height());
    assert_eq!(rows.last().unwrap().end_col(), 14);
    assert!(rows
        .windows(2)
        .all(|pair| pair[0].end_col() == pair[1].start_col));
    assert!(!scroll_viewport(&mut app, ScrollDirection::Down, MOUSE_WHEEL_ROWS).unwrap());
}

#[test]
fn resize_keeps_an_offscreen_cursor_decoupled_and_valid() {
    let mut app = app_with_lines(30);
    let mut out = Vec::new();
    app.buffer.set_cursor(Cursor { row: 0, col: 2 });
    handle_mouse_wheel(&mut app, &mut out, ScrollDirection::Down, 0).unwrap();
    assert_eq!(app.screen.scroll_top, 3);

    app.handle_resize(100, 8, &mut out).unwrap();

    assert_eq!(app.buffer.cursor(), Cursor { row: 0, col: 2 });
    assert_eq!(app.screen.scroll_top, 3);
    assert!(app.screen.scroll_top < app.buffer.line_count());
}

#[test]
fn resize_preserves_a_valid_wrapped_origin_with_an_offscreen_cursor() {
    let mut app = App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text(&"abcdefghij".repeat(8)));
    app.view.soft_wrap = true;
    app.screen.width = 4;
    app.screen.height = 4;
    let mut out = Vec::new();
    handle_mouse_wheel(&mut app, &mut out, ScrollDirection::Down, 0).unwrap();
    let before = app.screen.wrap_col;
    assert!(before > 0);

    app.handle_resize(5, 5, &mut out).unwrap();

    assert_eq!(app.buffer.cursor(), Cursor::default());
    assert_eq!(app.screen.scroll_top, 0);
    assert_eq!(app.screen.wrap_col, before);
    assert!(app.screen.wrap_col <= app.buffer.line_char_count(0).unwrap());
    assert!(String::from_utf8_lossy(&out).ends_with("\x1b[?25l\x1b[1;1H"));
}

#[test]
fn growing_a_wrapped_viewport_does_not_rebase_a_decoupled_logical_row() {
    let mut app = App::new(None).unwrap();
    app.buffer = Box::new(PieceTable::from_text("abcdefgh\nijklmnop\nqrstuvwx"));
    app.view.soft_wrap = true;
    app.screen.width = 4;
    app.screen.height = 4;
    let mut out = Vec::new();
    handle_mouse_wheel(&mut app, &mut out, ScrollDirection::Down, 0).unwrap();
    let origin = (app.screen.scroll_top, app.screen.wrap_col);
    assert_eq!(origin, (1, 4));

    app.handle_resize(4, 10, &mut out).unwrap();

    assert_eq!((app.screen.scroll_top, app.screen.wrap_col), origin);
    assert_eq!(app.buffer.cursor(), Cursor::default());
}

#[test]
fn wheel_scrolls_help_without_moving_help_or_source_cursors() {
    let mut app = app_with_lines(30);
    app.screen.height = 5;
    app.view.soft_wrap = true;
    app.screen.wrap_col = 7;
    app.buffer.set_cursor(Cursor { row: 2, col: 3 });
    let source_cursor = app.buffer.cursor();
    let source_history = app.buffer.edit_history_position();
    let mut out = Vec::new();
    crate::app::help::show(&mut app, &mut out).unwrap();
    let help_cursor = crate::app::view::display_buffer(&app).cursor();

    handle_mouse_wheel(&mut app, &mut out, ScrollDirection::Down, 0).unwrap();

    assert_eq!(app.screen.scroll_top, MOUSE_WHEEL_ROWS);
    assert_eq!(crate::app::view::display_buffer(&app).cursor(), help_cursor);
    assert_eq!(app.buffer.cursor(), source_cursor);
    assert_eq!(app.buffer.edit_history_position(), source_history);
    assert_eq!(app.screen.wrap_col, 0, "help owns its temporary viewport");

    crate::app::help::handle_key(
        &mut app,
        &mut out,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    )
    .unwrap();
    assert_eq!(app.screen.wrap_col, 7);
}

#[test]
fn wheel_scrolls_markdown_and_proposal_views_without_source_mutation() {
    let mut markdown = App::new(None).unwrap();
    markdown.file.path = Some("notes.md".into());
    markdown.buffer = Box::new(PieceTable::from_text(
        "# Title\n\n- one\n- two\n- three\n- four\n- five",
    ));
    markdown.screen.height = 4;
    let markdown_source = markdown.buffer.to_string();
    let markdown_history = markdown.buffer.edit_history_position();
    let mut out = Vec::new();
    crate::app::view::handle_key(
        &mut markdown,
        &mut out,
        KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE),
    )
    .unwrap();
    let preview_cursor = crate::app::view::display_buffer(&markdown).cursor();
    handle_mouse_wheel(&mut markdown, &mut out, ScrollDirection::Down, 0).unwrap();
    assert!(markdown.screen.scroll_top > 0);
    assert_eq!(
        crate::app::view::display_buffer(&markdown).cursor(),
        preview_cursor
    );
    assert_eq!(markdown.buffer.to_string(), markdown_source);
    assert_eq!(markdown.buffer.edit_history_position(), markdown_history);

    let mut proposal = App::new(None).unwrap();
    proposal.buffer = Box::new(PieceTable::from_text("one\ntwo\n"));
    proposal.screen.height = 3;
    let proposal_source = proposal.buffer.to_string();
    let proposal_history = proposal.buffer.edit_history_position();
    let patch = "--- a/note.txt\n+++ b/note.txt\n@@ -1,2 +1,2 @@\n one\n-two\n+TWO\n";
    crate::app::llm_preview::show(&mut proposal, &mut out, patch).unwrap();
    let diff_cursor = crate::app::view::display_buffer(&proposal).cursor();
    handle_mouse_wheel(&mut proposal, &mut out, ScrollDirection::Down, 0).unwrap();
    assert!(proposal.screen.scroll_top > 0);
    assert_eq!(
        crate::app::view::display_buffer(&proposal).cursor(),
        diff_cursor
    );
    assert_eq!(proposal.buffer.to_string(), proposal_source);
    assert_eq!(proposal.buffer.edit_history_position(), proposal_history);
    assert!(!proposal.file.dirty);
}

#[test]
fn wheel_over_status_or_an_active_prompt_is_ignored() {
    let mut app = app_with_lines(30);
    let mut out = Vec::new();
    let status_row = app.screen.visible_height();
    handle_mouse_wheel(&mut app, &mut out, ScrollDirection::Down, status_row).unwrap();
    assert_eq!(app.screen.scroll_top, 0);

    crate::app::search::open_prompt(&mut app, &mut out).unwrap();
    handle_mouse_wheel(&mut app, &mut out, ScrollDirection::Down, 0).unwrap();
    assert_eq!(app.screen.scroll_top, 0);
}
