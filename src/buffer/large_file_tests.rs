//! Tests for the read-only LargeFileBuffer.
//!
//! Purpose: cover bounded query/movement behavior for the Phase 2B Huge-file buffer.
//! Owns: LargeFileBuffer construction/query/movement/invalid UTF-8 tests.
//! Must not: allocate 10/100 MiB fixtures, assert timing, or test App policy.
//! Invariants: sibling test module of large_file.rs; uses tiny temp files only.
//! Phase: 2B limited Huge-file storage foundation.

#![cfg(test)]

use super::*;

fn temp_path(name: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "catomic_large_file_buffer_{}_{}",
        std::process::id(),
        name
    ));
    p
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
}

#[test]
fn opens_and_queries_utf8_lines_without_editing() {
    let path = temp_path("utf8.txt");
    cleanup(&path);
    std::fs::write(&path, "hello\né猫🙂\nlast").unwrap();

    let mut buffer = LargeFileBuffer::open(&path).unwrap();

    assert!(buffer.is_read_only());
    assert_eq!(buffer.line_count(), 3);
    assert_eq!(buffer.line(0).as_deref(), Some("hello"));
    assert_eq!(buffer.line(1).as_deref(), Some("é猫🙂"));
    assert_eq!(buffer.line_char_count(1), Some(3));
    assert_eq!(buffer.visible_lines_window(1, 1, 1, 2)[0].content, "猫🙂");
    buffer.insert_char('x');
    assert_eq!(buffer.line(0).as_deref(), Some("hello"));
    assert_eq!(buffer.edit_history_position(), 0);

    cleanup(&path);
}

#[test]
fn opens_utf8_split_across_scan_chunk_boundary() {
    let path = temp_path("chunk_boundary.txt");
    cleanup(&path);
    let prefix = "a".repeat(SCAN_CHUNK_BYTES - 1);
    std::fs::write(&path, format!("{}🙂\nnext", prefix)).unwrap();

    let buffer = LargeFileBuffer::open(&path).unwrap();

    assert_eq!(buffer.line_count(), 2);
    assert_eq!(buffer.line_char_count(0), Some(SCAN_CHUNK_BYTES));
    assert_eq!(
        buffer.visible_lines_window(0, 1, SCAN_CHUNK_BYTES - 2, 3)[0].content,
        "a🙂"
    );
    assert_eq!(buffer.line(1).as_deref(), Some("next"));

    cleanup(&path);
}

#[test]
fn records_ascii_lines_and_windows_late_ascii_columns() {
    let path = temp_path("ascii_window.txt");
    cleanup(&path);
    let line = "0123456789".repeat(8_000);
    std::fs::write(&path, format!("{}\né猫\nplain", line)).unwrap();

    let buffer = LargeFileBuffer::open(&path).unwrap();

    assert_eq!(buffer.line_is_ascii, vec![true, false, true]);
    assert_eq!(buffer.line_char_count(0), Some(80_000));
    assert_eq!(
        buffer.visible_lines_window(0, 1, 79_990, 10)[0].content,
        "0123456789"
    );
    assert_eq!(buffer.visible_lines_window(1, 1, 1, 1)[0].content, "猫");

    cleanup(&path);
}

#[test]
fn movement_clamps_to_line_char_counts() {
    let path = temp_path("movement.txt");
    cleanup(&path);
    std::fs::write(&path, "abcd\né\nxyz").unwrap();

    let mut buffer = LargeFileBuffer::open(&path).unwrap();
    for _ in 0..4 {
        buffer.move_right();
    }
    assert_eq!(buffer.cursor(), Cursor { row: 0, col: 4 });
    buffer.move_right();
    assert_eq!(buffer.cursor(), Cursor { row: 1, col: 0 });
    buffer.move_down();
    assert_eq!(buffer.cursor(), Cursor { row: 2, col: 0 });
    buffer.move_right();
    buffer.move_right();
    buffer.move_up();
    assert_eq!(buffer.cursor(), Cursor { row: 1, col: 1 });

    cleanup(&path);
}

#[test]
fn invalid_utf8_is_rejected_at_open() {
    let path = temp_path("invalid.bin");
    cleanup(&path);
    std::fs::write(&path, [0xff, b'\n']).unwrap();

    let err = match LargeFileBuffer::open(&path) {
        Ok(_) => panic!("invalid UTF-8 must fail"),
        Err(err) => err,
    };

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    cleanup(&path);
}
