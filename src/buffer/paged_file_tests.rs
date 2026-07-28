//! Purpose: verify editable paged storage, cross-page history, and whole-file output.
//! Owns: small deterministic tests for retained edits and original-range overlays.
//! Must not: depend on App policy, terminal input, live watchers, or large fixtures.
//! Invariants: configured pages stay bounded; every page remains editable and writable.

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
fn paged_typing_run_has_one_undo_and_distinct_content_revisions() {
    let path = temp_path("grouped_revision");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    buffer.insert_char('a');
    let history = buffer.edit_history_position();
    let first_revision = buffer.content_revision();
    buffer.insert_char('b');
    let refreshed_history = buffer.edit_history_position();
    let second_revision = buffer.content_revision();

    assert_ne!(refreshed_history, history);
    assert_ne!(buffer.content_revision(), first_revision);
    buffer.undo();
    assert_eq!(buffer.line(0).as_deref(), Some("zero"));
    assert_eq!(buffer.edit_history_position(), 0);
    let undo_revision = buffer.content_revision();

    buffer.redo();
    assert_eq!(buffer.line(0).as_deref(), Some("abzero"));
    assert_eq!(
        buffer.edit_history_position(),
        refreshed_history,
        "redo restores the refreshed token for the complete typing run"
    );
    assert_ne!(buffer.content_revision(), undo_revision);
    assert_ne!(second_revision, undo_revision);

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

#[test]
fn crlf_page_navigation_hides_synthetic_rows_at_source_boundaries() {
    let path = temp_path("crlf_navigation");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\r\n\r\ntwo\r\n猫").unwrap();

    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    assert_eq!(buffer.lines(), vec!["zero", ""]);
    let first = buffer.page_info().unwrap();
    assert_eq!(first.start_byte, 0);
    assert_eq!(first.end_byte, b"zero\r\n\r\n".len() as u64);
    assert!(first.has_next);

    assert!(buffer.next_page().unwrap());
    assert_eq!(buffer.lines(), vec!["two", "猫"]);
    let second = buffer.page_info().unwrap();
    assert_eq!(second.start_byte, first.end_byte);
    assert!(!second.has_next);

    assert!(buffer.previous_page().unwrap());
    assert_eq!(buffer.lines(), vec!["zero", ""]);

    let _ = std::fs::remove_file(path);
}

#[test]
fn new_cross_page_edit_invalidates_redo_and_releases_clean_inactive_page() {
    let path = temp_path("redo-release");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo").unwrap();
    let mut buffer = PagedFileBuffer::open(&path, 1).unwrap();

    buffer.insert_char('X');
    buffer.undo();
    assert!(buffer.next_page().unwrap());
    assert_eq!(buffer.retained_page_count_for_test(), 1);

    buffer.insert_char('Y');
    assert_eq!(
        buffer.retained_page_count_for_test(),
        0,
        "invalidated redo must not pin an otherwise clean inactive page"
    );
    buffer.redo();
    assert_eq!(buffer.line(0).as_deref(), Some("Yone"));
    assert!(buffer.previous_page().unwrap());
    assert_eq!(buffer.line(0).as_deref(), Some("zero"));

    let _ = std::fs::remove_file(path);
}

#[test]
fn paged_pruning_keeps_newest_global_undo_and_current_edited_pages() {
    let path = temp_path("bounded-history");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none\ntwo").unwrap();
    let mut buffer = PagedFileBuffer::open(&path, 1).unwrap();
    buffer.set_history_retention_for_test(1, usize::MAX);

    buffer.insert_char('X');
    let retained_floor = buffer.edit_history_position();
    assert!(buffer.next_page().unwrap());
    buffer.insert_char('Y');

    buffer.undo();
    assert_eq!(buffer.line(0).as_deref(), Some("one"));
    assert_eq!(buffer.edit_history_position(), retained_floor);
    buffer.undo();
    assert_eq!(
        buffer.edit_history_position(),
        retained_floor,
        "the older global transaction was pruned"
    );
    assert!(buffer.previous_page().unwrap());
    assert_eq!(
        buffer.line(0).as_deref(),
        Some("Xzero"),
        "a page with current edits remains resident after its undo is pruned"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn paged_active_run_growth_refreshes_and_prunes_only_older_transactions() {
    let path = temp_path("grouped-byte-retention");
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, "zero\none").unwrap();
    let mut buffer = PagedFileBuffer::open(&path, 2).unwrap();
    buffer.set_history_retention_for_test(10, usize::MAX);

    for ch in ['a', 'b'] {
        buffer.insert_char(ch);
        buffer.finish_undo_group();
    }
    buffer.insert_char('c');
    let first_token = buffer.edit_history_position();
    let first_revision = buffer.content_revision();
    let (transactions, retained_bytes) = buffer.history_retention_metrics_for_test();
    assert_eq!(transactions, 3);
    buffer.set_history_retention_for_test(10, retained_bytes + 128);

    for _ in 0..retained_bytes.saturating_add(1024) {
        buffer.insert_char('x');
    }

    let (transactions, retained_bytes_after) = buffer.history_retention_metrics_for_test();
    assert_eq!(transactions, 1);
    assert!(retained_bytes_after > retained_bytes + 128);
    assert_ne!(buffer.edit_history_position(), first_token);
    assert_ne!(buffer.content_revision(), first_revision);
    buffer.undo();
    assert_eq!(buffer.line(0).as_deref(), Some("abzero"));
    buffer.undo();
    assert_eq!(buffer.line(0).as_deref(), Some("abzero"));

    let _ = std::fs::remove_file(path);
}
