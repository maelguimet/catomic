//! Purpose: verify editable paged storage, cross-page history, and whole-file output.
//! Owns: small deterministic tests for retained edits and original-range overlays.
//! Must not: depend on App policy, terminal input, live watchers, or large fixtures.
//! Invariants: configured pages stay bounded; every page remains editable and writable.
//! Phase: 2-by editable paged-file storage acceptance.

use std::io::Write;

use super::{Buffer, Cursor, PagedFileBuffer};

fn temp_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "catomic_paged_edit_{label}_{}.txt",
        std::process::id()
    ))
}

#[test]
fn edits_on_multiple_pages_stream_as_one_document() {
    let path = temp_path("stream");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo\nthree").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    assert_eq!(buffer.line_count(), 2);
    assert_eq!(buffer.lines(), vec!["zero", "one"]);
    buffer.insert_char('X');
    assert!(buffer.next_page().unwrap());
    assert_eq!(buffer.lines(), vec!["two", "three"]);
    buffer.insert_char('Y');

    let mut written = Vec::new();
    buffer.write_to(&mut written).unwrap();
    assert_eq!(written, b"Xzero\none\nYtwo\nthree");

    assert!(buffer.previous_page().unwrap());
    assert_eq!(buffer.line(0).as_deref(), Some("Xzero"));

    let _ = std::fs::remove_file(path);
}

#[test]
fn undo_and_redo_follow_edit_order_across_pages() {
    let path = temp_path("history");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo\nthree").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    buffer.insert_char('X');
    let first_edit = buffer.edit_history_position();
    buffer.next_page().unwrap();
    buffer.insert_char('Y');
    let second_edit = buffer.edit_history_position();

    buffer.undo();
    assert_eq!(buffer.page_info().unwrap().page_number, 2);
    assert_eq!(buffer.line(0).as_deref(), Some("two"));
    assert_eq!(buffer.edit_history_position(), first_edit);
    buffer.undo();
    assert_eq!(buffer.page_info().unwrap().page_number, 1);
    assert_eq!(buffer.line(0).as_deref(), Some("zero"));
    assert_eq!(buffer.edit_history_position(), 0);

    buffer.redo();
    assert_eq!(buffer.line(0).as_deref(), Some("Xzero"));
    assert_eq!(buffer.edit_history_position(), first_edit);
    buffer.redo();
    assert_eq!(buffer.page_info().unwrap().page_number, 2);
    assert_eq!(buffer.line(0).as_deref(), Some("Ytwo"));
    assert_eq!(buffer.edit_history_position(), second_edit);

    let _ = std::fs::remove_file(path);
}

#[test]
fn backspace_at_page_start_removes_the_previous_page_boundary() {
    let path = temp_path("boundary");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    buffer.next_page().unwrap();
    buffer.set_cursor(Cursor { row: 0, col: 0 });
    buffer.delete_back();

    assert_eq!(buffer.page_info().unwrap().page_number, 1);
    let mut written = Vec::new();
    buffer.write_to(&mut written).unwrap();
    assert_eq!(written, b"zero\nonetwo");

    let _ = std::fs::remove_file(path);
}

#[test]
fn descriptor_drift_blocks_page_load_and_streaming() {
    let path = temp_path("drift");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 1).unwrap();
    let mut external = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    external.write_all(b"\nchanged").unwrap();
    external.sync_all().unwrap();

    assert!(buffer.next_page().is_err());
    assert!(buffer.write_to(&mut Vec::new()).is_err());
    assert_eq!(buffer.page_info().unwrap().page_number, 1);
    assert!(buffer.try_visible_lines_window(0, 1, 0, 80).is_err());

    let _ = std::fs::remove_file(path);
}

#[test]
fn range_replacement_is_one_paged_history_transaction() {
    let path = temp_path("range");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo").unwrap();
    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    buffer.set_cursor(Cursor { row: 1, col: 1 });

    assert!(buffer
        .replace_range(Cursor { row: 0, col: 2 }, Cursor { row: 1, col: 2 }, "X",)
        .unwrap());
    assert_eq!(buffer.lines(), vec!["zeXe"]);

    buffer.undo();
    assert_eq!(buffer.lines(), vec!["zero", "one"]);
    assert_eq!(buffer.cursor(), Cursor { row: 1, col: 1 });
    let _ = std::fs::remove_file(path);
}
