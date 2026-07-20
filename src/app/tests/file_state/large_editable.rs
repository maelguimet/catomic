//! Purpose: verify App editing, saving, and exact dirty state for paged files.
//! Owns: small deterministic App-level paged edit/save acceptance cases.
//! Must not: allocate threshold-sized fixtures, use live watchers, or bypass Buffer edits.
//! Invariants: every page is editable; Ctrl+S streams the complete logical document.
//! Phase: 2-bz editable oversized-file App integration.

use super::super::*;
use super::make_key;
use crossterm::event::{KeyCode, KeyModifiers};
use std::fs;

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "catomic_large_edit_{}_{}",
        std::process::id(),
        name
    ))
}

fn app_with_paged_buffer(path: &std::path::Path) -> App {
    app_with_configured_paged_buffer(path, |_| {})
}

fn app_with_configured_paged_buffer(
    path: &std::path::Path,
    configure: impl FnOnce(&mut crate::buffer::PagedFileBuffer),
) -> App {
    let mut app = App::new(None).unwrap();
    app.file.path = Some(path.to_path_buf());
    app.file.disk_snapshot = crate::file::io::capture_file_snapshot(path).ok();
    app.file.size_bytes = Some(crate::file::size::LARGE_FILE_LIMIT_BYTES + 1);
    app.file.size_tier = Some(crate::file::size::FileSizeTier::Huge);
    app.file.text_format = crate::file::text_format::detect_file_format(path).unwrap();
    let mut buffer = crate::buffer::PagedFileBuffer::open(path, 1).unwrap();
    configure(&mut buffer);
    app.buffer = Box::new(buffer);
    app.file.saved_history_position = app.buffer.edit_history_position();
    app
}

fn assert_save_failure_restored(
    app: &App,
    path: &std::path::Path,
    original: (crate::buffer::PageInfo, crate::buffer::Cursor),
    selection: crate::editor::selection::Selection,
    viewport: (usize, usize, usize),
    contents: &str,
    error: &str,
) {
    assert_eq!(app.buffer.page_info(), Some(original.0));
    assert_eq!(app.buffer.cursor(), original.1);
    assert_eq!(app.selection.active(), Some(selection));
    assert_eq!(
        (
            app.screen.scroll_top,
            app.screen.scroll_left,
            app.screen.wrap_col
        ),
        viewport
    );
    assert_eq!(fs::read_to_string(path).unwrap(), contents);
    assert!(app.message.as_deref().unwrap_or("").contains(error));
    assert!(!app.should_quit);
}

fn selected_save_state(app: &mut App) -> (
    (crate::buffer::PageInfo, crate::buffer::Cursor),
    crate::editor::selection::Selection,
    (usize, usize, usize),
) {
    app.buffer
        .set_cursor(crate::buffer::Cursor { row: 0, col: 1 });
    app.handle_key_with(
        &mut Vec::new(),
        make_key(KeyCode::Right, KeyModifiers::SHIFT),
    )
    .unwrap();
    app.screen.scroll_top = 7;
    app.screen.scroll_left = 8;
    app.screen.wrap_col = 9;
    (
        (app.buffer.page_info().unwrap(), app.buffer.cursor()),
        app.selection.active().unwrap(),
        (7, 8, 9),
    )
}

#[test]
fn paged_save_advance_failure_stays_in_editor_and_restores_state() {
    let path = temp_path("advance_failure.txt");
    let contents = "first\nsecond\nthird";
    let _ = fs::remove_file(&path);
    fs::write(&path, contents).unwrap();
    let mut app = app_with_configured_paged_buffer(&path, |buffer| {
        buffer.fail_next_page_after(1, std::io::ErrorKind::PermissionDenied);
    });
    let (original, selection, viewport) = selected_save_state(&mut app);

    app.handle_key_with(
        &mut Vec::new(),
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert_save_failure_restored(
        &app,
        &path,
        original,
        selection,
        viewport,
        contents,
        "permission denied",
    );
    assert!(!app.file.dirty);
    let _ = fs::remove_file(path);
}

#[test]
fn paged_save_return_failure_keeps_newline_dirty_and_restores_state() {
    let path = temp_path("return_failure.txt");
    let contents = "first\nsecond";
    let _ = fs::remove_file(&path);
    fs::write(&path, contents).unwrap();
    let mut app = app_with_configured_paged_buffer(&path, |buffer| {
        buffer.fail_previous_page_once(std::io::ErrorKind::TimedOut);
    });
    let (original, selection, viewport) = selected_save_state(&mut app);

    app.handle_key_with(
        &mut Vec::new(),
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert_save_failure_restored(
        &app,
        &path,
        original,
        selection,
        viewport,
        contents,
        "timed out",
    );
    assert!(app.file.dirty);
    assert_ne!(
        app.buffer.edit_history_position(),
        app.file.saved_history_position
    );
    let _ = fs::remove_file(path);
}

#[test]
fn paged_save_preserves_crlf_without_doubling_carriage_returns() {
    let path = temp_path("crlf_save.txt");
    let _ = fs::remove_file(&path);
    fs::write(&path, b"first\r\nsecond\r\n").unwrap();
    let mut app = app_with_paged_buffer(&path);
    let mut out = Vec::new();
    assert_eq!(app.buffer.line(0).unwrap(), "first");
    app.buffer
        .set_cursor(crate::buffer::Cursor { row: 0, col: 5 });

    app.handle_key_with(&mut out, make_key(KeyCode::Char('X'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert_eq!(fs::read(&path).unwrap(), b"firstX\r\nsecond\r\n");
    let _ = fs::remove_file(path);
}

#[test]
fn paged_buffer_edits_multiple_pages_and_saves_the_whole_file() {
    let path = temp_path("save.txt");
    let _ = fs::remove_file(&path);
    fs::write(&path, "first\nsecond").unwrap();
    let mut app = app_with_paged_buffer(&path);
    let mut out = Vec::new();

    app.handle_key_with(&mut out, make_key(KeyCode::Char('X'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(&mut out, make_key(KeyCode::PageDown, KeyModifiers::CONTROL))
        .unwrap();
    app.handle_key_with(&mut out, make_key(KeyCode::Char('Y'), KeyModifiers::NONE))
        .unwrap();

    assert!(app.file.dirty);
    assert_eq!(fs::read_to_string(&path).unwrap(), "first\nsecond");
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert_eq!(fs::read_to_string(&path).unwrap(), "Xfirst\nYsecond\n");
    assert!(!app.file.dirty);
    assert!(app.message.is_none());

    let _ = fs::remove_file(path);
}

#[test]
fn paged_buffer_keeps_editing_untouched_pages_after_atomic_save() {
    let path = temp_path("successive_save.txt");
    let _ = fs::remove_file(&path);
    fs::write(&path, "first\nsecond\nthird").unwrap();
    let mut app = app_with_paged_buffer(&path);
    let mut out = Vec::new();

    app.handle_key_with(&mut out, make_key(KeyCode::Char('X'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "Xfirst\nsecond\nthird\n"
    );

    app.handle_key_with(&mut out, make_key(KeyCode::PageDown, KeyModifiers::CONTROL))
        .unwrap();
    app.handle_key_with(&mut out, make_key(KeyCode::Char('Y'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();

    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "Xfirst\nYsecond\nthird\n"
    );
    assert!(!app.file.dirty);

    let _ = fs::remove_file(path);
}

#[test]
fn paged_buffer_undo_and_redo_track_the_saved_position_exactly() {
    let path = temp_path("dirty.txt");
    let _ = fs::remove_file(&path);
    fs::write(&path, "first\nsecond").unwrap();
    let mut app = app_with_paged_buffer(&path);
    let mut out = Vec::new();

    app.handle_key_with(&mut out, make_key(KeyCode::Char('X'), KeyModifiers::NONE))
        .unwrap();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('s'), KeyModifiers::CONTROL),
    )
    .unwrap();
    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('z'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert!(app.file.dirty);

    app.handle_key_with(
        &mut out,
        make_key(KeyCode::Char('y'), KeyModifiers::CONTROL),
    )
    .unwrap();
    assert!(!app.file.dirty);

    let _ = fs::remove_file(path);
}
