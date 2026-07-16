//! Tests for the read-only LargeFileBuffer.
//!
//! Purpose: cover bounded query/movement behavior for the Phase 2B Huge-file buffer.
//! Owns: LargeFileBuffer construction/query/movement/invalid UTF-8 tests.
//! Must not: allocate 10/100 MiB fixtures, assert timing, or test App policy.
//! Invariants: sibling test module of large_file.rs; uses tiny temp files only.
//! Phase: 2B limited Huge-file storage foundation.

#![cfg(test)]

use super::*;
use crate::buffer::Buffer;
use std::io::Write;
use std::path::{Path, PathBuf};

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
fn records_checkpoints_for_late_non_ascii_windows() {
    let path = temp_path("non_ascii_checkpoint.txt");
    cleanup(&path);
    let line = "é".repeat((LINE_CHECKPOINT_INTERVAL_CHARS * 2) + 5);
    std::fs::write(&path, format!("{}\nend", line)).unwrap();

    let buffer = LargeFileBuffer::open(&path).unwrap();

    assert_eq!(buffer.line_is_ascii[0], false);
    assert_eq!(
        buffer.line_checkpoints(0)[0],
        LineCheckpoint {
            col: LINE_CHECKPOINT_INTERVAL_CHARS,
            byte_offset: LINE_CHECKPOINT_INTERVAL_CHARS * 2,
        }
    );
    assert_eq!(
        buffer.line_checkpoint_at_or_before(0, (LINE_CHECKPOINT_INTERVAL_CHARS * 2) + 3),
        Some(LineCheckpoint {
            col: LINE_CHECKPOINT_INTERVAL_CHARS * 2,
            byte_offset: LINE_CHECKPOINT_INTERVAL_CHARS * 4,
        })
    );
    assert_eq!(
        buffer.visible_lines_window(0, 1, (LINE_CHECKPOINT_INTERVAL_CHARS * 2) + 1, 3)[0].content,
        "ééé"
    );

    cleanup(&path);
}

#[test]
fn records_ascii_prefix_checkpoints_for_later_non_ascii_lines() {
    let path = temp_path("ascii_prefix_checkpoint.txt");
    cleanup(&path);
    let prefix = "a".repeat(SCAN_CHUNK_BYTES + LINE_CHECKPOINT_INTERVAL_CHARS + 7);
    std::fs::write(&path, format!("{}éfin", prefix)).unwrap();

    let buffer = LargeFileBuffer::open(&path).unwrap();

    assert_eq!(buffer.line_is_ascii[0], false);
    assert_eq!(
        buffer.line_checkpoint_at_or_before(0, SCAN_CHUNK_BYTES + 3),
        Some(LineCheckpoint {
            col: SCAN_CHUNK_BYTES,
            byte_offset: SCAN_CHUNK_BYTES,
        })
    );
    assert_eq!(
        buffer.visible_lines_window(0, 1, prefix.len(), 4)[0].content,
        "éfin"
    );

    cleanup(&path);
}

#[test]
fn path_replacement_after_open_keeps_original_descriptor() {
    let path = temp_path("replace_target.txt");
    let replacement = temp_path("replace_new.txt");
    cleanup(&path);
    cleanup(&replacement);
    std::fs::write(&path, "old stable content").unwrap();

    let buffer = LargeFileBuffer::open(&path).unwrap();
    std::fs::write(&replacement, "new path content").unwrap();
    std::fs::rename(&replacement, &path).unwrap();

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new path content");
    assert_eq!(buffer.line(0).as_deref(), Some("old stable content"));
    assert_eq!(buffer.to_string(), "old stable content");
    assert_eq!(buffer.visible_lines_window(0, 1, 4, 6)[0].content, "stable");

    cleanup(&path);
    cleanup(&replacement);
}

#[test]
fn streaming_write_copies_descriptor_content() {
    let path = temp_path("stream_copy.txt");
    cleanup(&path);
    std::fs::write(&path, "first\né猫🙂\nlast").unwrap();
    let buffer = LargeFileBuffer::open(&path).unwrap();

    let mut written = Vec::new();
    buffer.write_to(&mut written).expect("stream descriptor");

    assert_eq!(written, std::fs::read(&path).unwrap());
    cleanup(&path);
}

#[test]
fn in_place_metadata_change_blocks_descriptor_reads() {
    let path = temp_path("in_place_change.txt");
    cleanup(&path);
    std::fs::write(&path, "original stable content").unwrap();

    let buffer = LargeFileBuffer::open(&path).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&path)
        .unwrap();
    file.write_all(b"changed").unwrap();
    file.flush().unwrap();
    drop(file);

    let err = buffer.read_range_to_string(0, 4).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    let window_err = buffer
        .try_visible_lines_window(0, 1, 0, 8)
        .expect_err("visible reads must surface a changed descriptor");
    assert_eq!(window_err.kind(), io::ErrorKind::InvalidData);
    let mut streamed = Vec::new();
    let stream_err = buffer
        .write_to(&mut streamed)
        .expect_err("streaming save must reject descriptor drift");
    assert_eq!(stream_err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(buffer.line(0).as_deref(), Some(""));
    assert_eq!(buffer.visible_lines_window(0, 1, 0, 8)[0].content, "");
    assert_eq!(buffer.to_string(), "");

    cleanup(&path);
}

#[test]
fn fallible_visible_window_checks_descriptor_before_and_after() {
    let path = temp_path("single_window_check.txt");
    cleanup(&path);
    std::fs::write(&path, "first\nsecond\nthird\nfourth").unwrap();

    let buffer = LargeFileBuffer::open_paged(&path, 4).unwrap();
    let lines = buffer
        .try_visible_lines_window(0, 4, 0, 80)
        .expect("stable descriptor window");

    assert_eq!(
        lines
            .iter()
            .map(|line| line.content.as_str())
            .collect::<Vec<_>>(),
        vec!["first", "second", "third", "fourth"]
    );
    assert_eq!(buffer.metadata_check_count(), 2);

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

#[test]
fn paged_open_retains_only_configured_lines_and_navigates_both_directions() {
    let path = temp_path("pages.txt");
    cleanup(&path);
    std::fs::write(&path, "zero\none\ntwo\nthree\nfour").unwrap();

    let mut buffer = LargeFileBuffer::open_paged(&path, 2).unwrap();
    assert_eq!(buffer.lines(), vec!["zero", "one"]);
    assert_eq!(buffer.page_info().unwrap().page_number, 1);
    assert!(buffer.next_page().unwrap());
    assert_eq!(buffer.lines(), vec!["two", "three"]);
    assert_eq!(buffer.page_info().unwrap().page_number, 2);
    assert!(buffer.next_page().unwrap());
    assert_eq!(buffer.lines(), vec!["four"]);
    assert!(!buffer.next_page().unwrap());
    assert!(buffer.previous_page().unwrap());
    assert_eq!(buffer.lines(), vec!["two", "three"]);
    assert!(buffer.previous_page().unwrap());
    assert_eq!(buffer.lines(), vec!["zero", "one"]);
    assert!(!buffer.previous_page().unwrap());

    cleanup(&path);
}

#[test]
fn paged_open_exposes_trailing_empty_line_as_final_page() {
    let path = temp_path("trailing_page.txt");
    cleanup(&path);
    std::fs::write(&path, "zero\none\n").unwrap();

    let mut buffer = LargeFileBuffer::open_paged(&path, 2).unwrap();
    assert_eq!(buffer.lines(), vec!["zero", "one"]);
    assert!(buffer.next_page().unwrap());
    assert_eq!(buffer.lines(), vec![""]);
    assert!(!buffer.next_page().unwrap());

    cleanup(&path);
}

#[test]
fn paged_ascii_scan_preserves_multi_chunk_metadata_and_boundary() {
    let path = temp_path("paged_ascii_chunks.txt");
    cleanup(&path);
    let first = "a".repeat((SCAN_CHUNK_BYTES * 2) + 7);
    std::fs::write(&path, format!("{first}\nnext")).unwrap();

    let mut buffer = LargeFileBuffer::open_paged(&path, 1).unwrap();

    assert_eq!(buffer.line_char_count(0), Some(first.len()));
    assert_eq!(buffer.page_info().unwrap().end_byte, first.len() as u64 + 1);
    assert_eq!(
        buffer
            .line_checkpoint_at_or_before(0, first.len())
            .unwrap()
            .col,
        LINE_CHECKPOINT_INTERVAL_CHARS * 8
    );
    assert_eq!(
        buffer.visible_lines_window(0, 1, first.len() - 4, 4)[0].content,
        "aaaa"
    );
    assert!(buffer.next_page().unwrap());
    assert_eq!(buffer.line(0).as_deref(), Some("next"));

    cleanup(&path);
}

#[test]
fn previous_page_is_computed_after_a_direct_later_page_state() {
    let path = temp_path("reverse_pages.txt");
    cleanup(&path);
    std::fs::write(&path, "zero\none\ntwo\nthree\nfour").unwrap();

    let mut buffer = LargeFileBuffer::open_paged(&path, 1).unwrap();
    for _ in 0..4 {
        assert!(buffer.next_page().unwrap());
    }
    assert_eq!(buffer.lines(), vec!["four"]);
    for expected in ["three", "two", "one", "zero"] {
        assert!(buffer.previous_page().unwrap());
        assert_eq!(buffer.line(0).as_deref(), Some(expected));
    }
    assert!(!buffer.previous_page().unwrap());

    cleanup(&path);
}

#[test]
fn descriptor_drift_blocks_loading_the_next_page() {
    let path = temp_path("page_drift.txt");
    cleanup(&path);
    std::fs::write(&path, "zero\none\ntwo").unwrap();

    let mut buffer = LargeFileBuffer::open_paged(&path, 2).unwrap();
    let mut changed = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    changed.write_all(b"\nchanged").unwrap();
    changed.sync_all().unwrap();

    assert!(buffer.next_page().is_err());
    assert_eq!(buffer.lines(), vec!["", ""]);

    cleanup(&path);
}
